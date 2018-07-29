#![cfg_attr(feature = "cargo-clippy", allow(needless_pass_by_value))]

use super::compact_block::{short_transaction_id, short_transaction_id_keys, CompactBlock};
use bigint::H256;
use block_process::BlockProcess;
use ckb_chain::chain::ChainProvider;
use ckb_protocol;
use core::block::{Block, IndexedBlock};
use core::transaction::Transaction;
use fnv::{FnvHashMap, FnvHashSet};
use futures::future;
use futures::future::lazy;
use futures::sync::mpsc;
use getdata_process::GetDataProcess;
use getheaders_process::GetHeadersProcess;
use headers_process::HeadersProcess;
use network::NetworkContextExt;
use network::{NetworkContext, NetworkProtocolHandler, PeerId, TimerToken};
use pool::txs_pool::TransactionPool;
use protobuf;
use protobuf::RepeatedField;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use synchronizer::Synchronizer;
use tokio;
use tokio::prelude::*;
use util::Mutex;

pub const SEND_GET_HEADERS_TOKEN: TimerToken = 1;
pub const BLOCK_FETCH_TOKEN: TimerToken = 2;

pub enum Task {
    SendGetHeadersToAll(Box<NetworkContext>),
    FetchBlock(Box<NetworkContext>),
    SendGetHeadersToPeer(Box<NetworkContext>, PeerId),
    HandleGetheaders(Box<NetworkContext>, PeerId, ckb_protocol::GetHeaders),
    HandleHeaders(Box<NetworkContext>, PeerId, ckb_protocol::Headers),
    HandleGetdata(Box<NetworkContext>, PeerId, ckb_protocol::GetData),
    // HandleCompactBlock(Box<NetworkContext>, PeerId, ckb_protocol::CompactBlock),
    HandleBlock(Box<NetworkContext>, PeerId, ckb_protocol::Block),
}

pub struct SyncProtocol<C> {
    pub synchronizer: Synchronizer<C>,
    pub receiver: Mutex<Option<mpsc::Receiver<Task>>>,
    pub sender: mpsc::Sender<Task>,
}

impl<C: ChainProvider + 'static> SyncProtocol<C> {
    pub fn new(synchronizer: Synchronizer<C>) -> Self {
        let (sender, receiver) = mpsc::channel(65535);
        SyncProtocol {
            synchronizer,
            sender,
            receiver: Mutex::new(Some(receiver)),
        }
    }

    pub fn start(&self) {
        let receiver = self.receiver.lock().take().expect("start once");
        let synchronizer = self.synchronizer.clone();
        let handler = receiver.for_each(move |task| {
            let synchronizer = synchronizer.clone();
            match task {
                Task::SendGetHeadersToAll(nc) => tokio::spawn(lazy(move || {
                    Self::send_getheaders_to_all(synchronizer, nc);
                    future::ok(())
                })),
                Task::SendGetHeadersToPeer(nc, peer) => tokio::spawn(lazy(move || {
                    Self::send_getheaders_to_peer(synchronizer, nc.as_ref(), peer);
                    future::ok(())
                })),
                Task::HandleGetheaders(nc, peer, message) => tokio::spawn(lazy(move || {
                    Self::handle_getheaders(synchronizer, nc, peer, &message);
                    future::ok(())
                })),
                Task::HandleHeaders(nc, peer, message) => tokio::spawn(lazy(move || {
                    Self::handle_headers(synchronizer, nc, peer, &message);
                    future::ok(())
                })),
                Task::HandleGetdata(nc, peer, message) => tokio::spawn(lazy(move || {
                    Self::handle_getdata(synchronizer, nc, peer, &message);
                    future::ok(())
                })),
                Task::HandleBlock(nc, peer, message) => tokio::spawn(lazy(move || {
                    Self::handle_block(synchronizer, nc, peer, &message);
                    future::ok(())
                })),
                Task::FetchBlock(nc) => tokio::spawn(lazy(move || {
                    Self::find_blocks_to_fetch(synchronizer, nc);
                    future::ok(())
                })),
                // Task::HandleCompactBlock(nc, peer, message) => tokio::spawn(lazy(move || {
                //     Self::handle_cmpt_block(synchronizer, nc, peer, &message);
                //     future::ok(())
                // })),
            }
        });
        tokio::run(handler);
    }

    pub fn handle_getheaders(
        synchronizer: Synchronizer<C>,
        nc: Box<NetworkContext>,
        peer: PeerId,
        message: &ckb_protocol::GetHeaders,
    ) {
        GetHeadersProcess::new(message, &synchronizer, &peer, nc.as_ref()).execute()
    }

    pub fn handle_headers(
        synchronizer: Synchronizer<C>,
        nc: Box<NetworkContext>,
        peer: PeerId,
        message: &ckb_protocol::Headers,
    ) {
        HeadersProcess::new(message, &synchronizer, &peer, nc.as_ref()).execute()
    }

    fn handle_getdata(
        synchronizer: Synchronizer<C>,
        nc: Box<NetworkContext>,
        peer: PeerId,
        message: &ckb_protocol::GetData,
    ) {
        GetDataProcess::new(message, &synchronizer, &peer, nc.as_ref()).execute()
    }

    // fn handle_cmpt_block(
    //     synchronizer: Synchronizer<C>,
    //     nc: Box<NetworkContext>,
    //     peer: PeerId,
    //     message: &ckb_protocol::CompactBlock,
    // ) {
    //     CompactBlockProcess::new(message, &synchronizer, &peer, nc.as_ref()).execute()
    // }

    fn handle_block(
        synchronizer: Synchronizer<C>,
        nc: Box<NetworkContext>,
        peer: PeerId,
        message: &ckb_protocol::Block,
    ) {
        BlockProcess::new(message, &synchronizer, &peer, nc.as_ref()).execute()
    }

    pub fn find_blocks_to_fetch(synchronizer: Synchronizer<C>, nc: Box<NetworkContext>) {
        let peers: Vec<PeerId> = { synchronizer.peers.state.read().keys().cloned().collect() };
        info!(target: "sync", "poll find_blocks_to_fetch select peers");
        for peer in peers {
            let ret = synchronizer.get_blocks_to_fetch(peer);
            if let Some(v_fetch) = ret {
                Self::send_block_getdata(&v_fetch, nc.as_ref(), peer);
            }
        }
    }

    fn send_block_getdata(v_fetch: &[H256], nc: &NetworkContext, peer: PeerId) {
        let mut payload = ckb_protocol::Payload::new();
        let mut getdata = ckb_protocol::GetData::new();
        let inventory = v_fetch
            .iter()
            .map(|h| {
                let mut inventory = ckb_protocol::Inventory::new();
                inventory.set_inv_type(ckb_protocol::InventoryType::MSG_BLOCK);
                inventory.set_hash(h.to_vec());
                inventory
            })
            .collect();
        getdata.set_inventory(RepeatedField::from_vec(inventory));
        payload.set_getdata(getdata);

        let _ = nc.send_payload(peer, payload);
        debug!(target: "sync", "send_block_getdata len={:?} to peer={:?}", v_fetch.len() , peer);
    }

    fn send_getheaders_to_all(synchronizer: Synchronizer<C>, nc: Box<NetworkContext>) {
        let peers: Vec<PeerId> = { synchronizer.peers.state.read().keys().cloned().collect() };
        debug!(target: "sync", "send_getheaders to peers= {:?}", &peers);
        for peer in peers {
            Self::send_getheaders_to_peer(synchronizer.clone(), nc.as_ref(), peer);
        }
    }

    fn send_getheaders_to_peer(synchronizer: Synchronizer<C>, nc: &NetworkContext, peer: PeerId) {
        // TODO: set timeout
        synchronizer.n_sync.fetch_add(1, Ordering::Release);
        let tip = synchronizer.tip_header();
        let locator_hash = synchronizer.get_locator(&tip);
        let mut payload = ckb_protocol::Payload::new();
        let mut getheaders = ckb_protocol::GetHeaders::new();
        let locator_hash = locator_hash.into_iter().map(|hash| hash.to_vec()).collect();
        getheaders.set_version(0);
        getheaders.set_block_locator_hashes(RepeatedField::from_vec(locator_hash));
        getheaders.set_hash_stop(H256::zero().to_vec());
        payload.set_getheaders(getheaders);
        let _ = nc.send_payload(peer, payload);
        debug!(target: "sync", "send_getheaders_to_peer getheaders {:?} to peer={:?}", tip.number ,peer);
    }

    fn dispatch_getheaders(&self, nc: Box<NetworkContext>) {
        if self.synchronizer.n_sync.load(Ordering::Acquire) == 0
            || !self.synchronizer.is_initial_block_download()
        {
            debug!(target: "sync", "dispatch_getheaders");
            let mut sender = self.sender.clone();
            let ret = sender.try_send(Task::SendGetHeadersToAll(nc));

            if ret.is_err() {
                error!(target: "sync", "dispatch_getheaders peer error {:?}", ret);
            }
        }
    }

    fn dispatch_block_fetch(&self, nc: Box<NetworkContext>) {
        debug!(target: "sync", "dispatch_block_download");
        let mut sender = self.sender.clone();
        let ret = sender.try_send(Task::FetchBlock(nc));

        if ret.is_err() {
            error!(target: "sync", "dispatch_block_download peer error {:?}", ret);
        }
    }

    fn init_getheaders(&self, nc: Box<NetworkContext>, peer: PeerId) {
        if self.synchronizer.n_sync.load(Ordering::Acquire) == 0
            || !self.synchronizer.is_initial_block_download()
        {
            debug!(target: "sync", "init_getheaders peer={:?} connected", peer);
            let mut sender = self.sender.clone();
            let ret = sender.try_send(Task::SendGetHeadersToPeer(nc, peer));

            if ret.is_err() {
                error!(target: "sync", "init_getheaders peer={:?} error {:?}", peer, ret);
            }
        }
    }

    fn process(&self, nc: Box<NetworkContext>, peer: &PeerId, mut payload: ckb_protocol::Payload) {
        let mut sender = self.sender.clone();

        let ret = if payload.has_getheaders() {
            sender.try_send(Task::HandleGetheaders(nc, *peer, payload.take_getheaders()))
        } else if payload.has_headers() {
            let headers = payload.take_headers();
            debug!(target: "sync", "receive headers massge {}", headers.headers.len());
            sender.try_send(Task::HandleHeaders(nc, *peer, headers))
        } else if payload.has_getdata() {
            sender.try_send(Task::HandleGetdata(nc, *peer, payload.take_getdata()))
        } else if payload.has_block() {
            sender.try_send(Task::HandleBlock(nc, *peer, payload.take_block()))
        } else {
            Ok(())
        };

        if ret.is_err() {
            error!(target: "sync", "NetworkProtocolHandler dispatch message error {:?}", ret);
        }
    }
}

impl<C: ChainProvider + 'static> NetworkProtocolHandler for SyncProtocol<C> {
    fn initialize(&self, nc: Box<NetworkContext>) {
        // NOTE: 100ms is what bitcoin use.
        let _ = nc.register_timer(SEND_GET_HEADERS_TOKEN, Duration::from_millis(100));
        let _ = nc.register_timer(BLOCK_FETCH_TOKEN, Duration::from_millis(100));
    }

    /// Called when new network packet received.
    fn read(&self, nc: Box<NetworkContext>, peer: &PeerId, _packet_id: u8, data: &[u8]) {
        match protobuf::parse_from_bytes::<ckb_protocol::Payload>(data) {
            Ok(payload) => self.process(nc, peer, payload),
            Err(err) => warn!(target: "sync", "Failed to parse protobuf, error={:?}", err),
        };
    }

    fn connected(&self, nc: Box<NetworkContext>, peer: &PeerId) {
        info!(target: "sync", "peer={} SyncProtocol.connected", peer);
        self.synchronizer.peers.connected(peer);
        self.init_getheaders(nc, *peer);
    }

    fn disconnected(&self, _nc: Box<NetworkContext>, peer: &PeerId) {
        info!(target: "sync", "peer={} SyncProtocol.disconnected", peer);
        self.synchronizer.peers.disconnected(&peer);
    }

    fn timeout(&self, nc: Box<NetworkContext>, token: TimerToken) {
        if !self.synchronizer.peers.state.read().is_empty() {
            match token as usize {
                SEND_GET_HEADERS_TOKEN => self.dispatch_getheaders(nc),
                BLOCK_FETCH_TOKEN => self.dispatch_block_fetch(nc),
                _ => unreachable!(),
            }
        } else {
            debug!(target: "sync", "no peers connected");
        }
    }
}

pub struct RelayProtocol<C> {
    pub synchronizer: Synchronizer<C>,
    pub tx_pool: Arc<TransactionPool<C>>,
    // TODO add size limit or use bloom filter
    pub received_blocks: Mutex<FnvHashSet<H256>>,
    pub received_transactions: Mutex<FnvHashSet<H256>>,
    pub pending_compact_blocks: Mutex<FnvHashMap<H256, CompactBlock>>,
}

impl<C: ChainProvider + 'static> RelayProtocol<C> {
    pub fn new(synchronizer: Synchronizer<C>, tx_pool: &Arc<TransactionPool<C>>) -> Self {
        RelayProtocol {
            synchronizer,
            tx_pool: Arc::clone(tx_pool),
            received_blocks: Mutex::new(FnvHashSet::default()),
            received_transactions: Mutex::new(FnvHashSet::default()),
            pending_compact_blocks: Mutex::new(FnvHashMap::default()),
        }
    }

    pub fn relay(&self, nc: &NetworkContext, source: PeerId, payload: &ckb_protocol::Payload) {
        let peer_ids = self
            .synchronizer
            .peers
            .state
            .read()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for (peer_id, _session) in nc.sessions(&peer_ids) {
            if peer_id != source {
                let _ = nc.send_payload(peer_id, payload.clone());
            }
        }
    }

    fn reconstruct_block(
        &self,
        compact_block: &CompactBlock,
        transactions: Vec<Transaction>,
    ) -> (Option<IndexedBlock>, Option<Vec<usize>>) {
        let (key0, key1) = short_transaction_id_keys(compact_block.nonce, &compact_block.header);

        let mut txs = transactions;
        txs.extend(self.tx_pool.pool.read().pool.get_vertices());
        txs.extend(self.tx_pool.orphan.read().pool.get_vertices());

        let mut txs_map = FnvHashMap::default();
        for tx in txs {
            let short_id = short_transaction_id(key0, key1, &tx.hash());
            txs_map.insert(short_id, tx);
        }

        let mut block_transactions = Vec::with_capacity(compact_block.short_ids.len());
        let mut missing_indexes = Vec::new();
        for (index, short_id) in compact_block.short_ids.iter().enumerate() {
            match txs_map.remove(short_id) {
                Some(tx) => block_transactions.insert(index, tx),
                None => missing_indexes.push(index),
            }
        }

        if missing_indexes.is_empty() {
            let block = Block::new(compact_block.header.clone(), block_transactions);

            (Some(block.into()), None)
        } else {
            (None, Some(missing_indexes))
        }
    }

    fn process(&self, nc: Box<NetworkContext>, peer: &PeerId, payload: ckb_protocol::Payload) {
        if payload.has_transaction() {
            let tx: Transaction = payload.get_transaction().into();
            if !self.received_transactions.lock().insert(tx.hash()) {
                let _ = self.tx_pool.add_to_memory_pool(tx);
                self.relay(nc.as_ref(), *peer, &payload);
            }
        } else if payload.has_block() {
            let block: Block = payload.get_block().into();
            if !self.received_blocks.lock().insert(block.hash()) {
                self.synchronizer.process_new_block(*peer, block.into());
                self.relay(nc.as_ref(), *peer, &payload);
            }
        } else if payload.has_compact_block() {
            let compact_block: CompactBlock = payload.get_compact_block().into();
            debug!(target: "sync", "receive compact block from peer#{}, {} => {}",
                   peer,
                   compact_block.header().number,
                   compact_block.header().hash(),
            );
            if !self
                .received_blocks
                .lock()
                .insert(compact_block.header.hash())
            {
                match self.reconstruct_block(&compact_block, Vec::new()) {
                    (Some(block), _) => {
                        self.synchronizer.process_new_block(*peer, block);
                        self.relay(nc.as_ref(), *peer, &payload);
                    }
                    (_, Some(missing_indexes)) => {
                        let mut payload = ckb_protocol::Payload::new();
                        let mut cbr = ckb_protocol::BlockTransactionsRequest::new();
                        cbr.set_hash(compact_block.header.hash().to_vec());
                        cbr.set_indexes(missing_indexes.into_iter().map(|i| i as u32).collect());
                        payload.set_block_transactions_request(cbr);
                        self.pending_compact_blocks
                            .lock()
                            .insert(compact_block.header.hash(), compact_block);
                        let _ = nc.respond_payload(payload);
                    }
                    (None, None) => {
                        // TODO fail to reconstruct block, downgrade to header first?
                    }
                }
            }
        } else if payload.has_block_transactions_request() {
            let btr = payload.get_block_transactions_request();
            let hash = H256::from_slice(btr.get_hash());
            let indexes = btr.get_indexes();
            if let Some(block) = self.synchronizer.get_block(&hash) {
                let mut payload = ckb_protocol::Payload::new();
                let mut bt = ckb_protocol::BlockTransactions::new();
                bt.set_hash(hash.to_vec());
                bt.set_transactions(RepeatedField::from_vec(
                    indexes
                        .iter()
                        .filter_map(|i| block.transactions.get(*i as usize))
                        .map(Into::into)
                        .collect(),
                ));
                let _ = nc.respond_payload(payload);
            }
        } else if payload.has_block_transactions() {
            let bt = payload.get_block_transactions();
            let hash = H256::from_slice(bt.get_hash());
            if let Some(compact_block) = self.pending_compact_blocks.lock().remove(&hash) {
                let transactions: Vec<Transaction> =
                    bt.get_transactions().iter().map(Into::into).collect();
                if let (Some(block), _) = self.reconstruct_block(&compact_block, transactions) {
                    self.synchronizer.process_new_block(*peer, block);
                }
            }
        }
    }
}

impl<C: ChainProvider + 'static> NetworkProtocolHandler for RelayProtocol<C> {
    /// Called when new network packet received.
    fn read(&self, nc: Box<NetworkContext>, peer: &PeerId, _packet_id: u8, data: &[u8]) {
        match protobuf::parse_from_bytes::<ckb_protocol::Payload>(data) {
            Ok(payload) => self.process(nc, peer, payload),
            Err(err) => warn!(target: "sync", "Failed to parse protobuf, error={:?}", err),
        };
    }

    fn connected(&self, _nc: Box<NetworkContext>, peer: &PeerId) {
        info!(target: "sync", "peer={} RelayProtocol.connected", peer);
        // do nothing
    }

    fn disconnected(&self, _nc: Box<NetworkContext>, peer: &PeerId) {
        info!(target: "sync", "peer={} RelayProtocol.disconnected", peer);
        // TODO
    }
}
