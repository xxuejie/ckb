use crate::{
    CellDataProvider, EpochProvider, ExtensionProvider, HeaderFields, HeaderFieldsProvider,
    HeaderProvider,
};
use ckb_types::{
    bytes::Bytes,
    core::{
        BlockExt, BlockNumber, BlockView, EpochExt, EpochNumber, HeaderView, TransactionInfo,
        TransactionView, UncleBlockVecView, cell::CellMeta,
    },
    packed::{self, Byte32, OutPoint},
};
use std::sync::Arc;

/// The `ChainStore` trait provides chain data store interface
pub trait ChainStore: Send + Sync + Sized {
    /// Return the borrowed data loader wrapper
    fn borrow_as_data_loader(&self) -> BorrowedDataLoaderWrapper<Self> {
        BorrowedDataLoaderWrapper::new(self)
    }

    /// Get block by block header hash
    fn get_block(&self, h: &packed::Byte32) -> Option<BlockView>;

    /// Get header by block header hash
    fn get_block_header(&self, hash: &packed::Byte32) -> Option<HeaderView>;

    /// Get block body by block header hash
    fn get_block_body(&self, hash: &packed::Byte32) -> Vec<TransactionView>;

    /// Get unfrozen block from ky-store with given hash
    fn get_unfrozen_block(&self, hash: &packed::Byte32) -> Option<BlockView>;

    /// Get all transaction-hashes in block body by block header hash
    fn get_block_txs_hashes(&self, hash: &packed::Byte32) -> Vec<packed::Byte32>;

    /// Get proposal short id by block header hash
    fn get_block_proposal_txs_ids(
        &self,
        hash: &packed::Byte32,
    ) -> Option<packed::ProposalShortIdVec>;

    /// Get block uncles by block header hash
    fn get_block_uncles(&self, hash: &packed::Byte32) -> Option<UncleBlockVecView>;

    /// Get block extension by block header hash
    fn get_block_extension(&self, hash: &packed::Byte32) -> Option<packed::Bytes>;

    /// Get block ext by block header hash
    ///
    /// Since v0.106, `BlockExt` added two option fields, so we have to use compatibility mode to read
    fn get_block_ext(&self, block_hash: &packed::Byte32) -> Option<BlockExt>;

    /// Get block header hash by block number
    fn get_block_hash(&self, number: BlockNumber) -> Option<packed::Byte32>;

    /// Get block number by block header hash
    fn get_block_number(&self, hash: &packed::Byte32) -> Option<BlockNumber>;

    /// TODO(doc): @quake
    fn is_main_chain(&self, hash: &packed::Byte32) -> bool;

    /// TODO(doc): @quake
    fn get_tip_header(&self) -> Option<HeaderView>;

    /// Returns true if the transaction confirmed in main chain.
    ///
    /// This function is base on transaction index `COLUMN_TRANSACTION_INFO`.
    /// Current release maintains a full index of historical transaction by default, this may be changed in future
    fn transaction_exists(&self, hash: &packed::Byte32) -> bool;

    /// Get commit transaction and block hash by its hash
    fn get_transaction(&self, hash: &packed::Byte32) -> Option<(TransactionView, packed::Byte32)>;

    /// TODO(doc): @quake
    fn get_transaction_info(&self, hash: &packed::Byte32) -> Option<TransactionInfo>;

    /// Gets transaction and associated info with correspond hash
    fn get_transaction_with_info(
        &self,
        hash: &packed::Byte32,
    ) -> Option<(TransactionView, TransactionInfo)>;

    /// Return whether cell is live
    fn have_cell(&self, out_point: &OutPoint) -> bool;

    /// Gets cell meta data with out_point
    fn get_cell(&self, out_point: &OutPoint) -> Option<CellMeta>;

    /// TODO(doc): @quake
    fn get_cell_data(&self, out_point: &OutPoint) -> Option<(Bytes, packed::Byte32)>;

    /// TODO(doc): @quake
    fn get_cell_data_hash(&self, out_point: &OutPoint) -> Option<packed::Byte32>;

    /// Gets current epoch ext
    fn get_current_epoch_ext(&self) -> Option<EpochExt>;

    /// Gets epoch ext by epoch index
    fn get_epoch_ext(&self, hash: &packed::Byte32) -> Option<EpochExt>;

    /// Gets epoch index by epoch number
    fn get_epoch_index(&self, number: EpochNumber) -> Option<packed::Byte32>;

    /// Gets epoch index by block hash
    fn get_block_epoch_index(&self, block_hash: &packed::Byte32) -> Option<packed::Byte32>;

    /// TODO(doc): @quake
    fn get_block_epoch(&self, hash: &packed::Byte32) -> Option<EpochExt>;

    /// TODO(doc): @quake
    fn is_uncle(&self, hash: &packed::Byte32) -> bool;

    /// Gets header by uncle header hash
    fn get_uncle_header(&self, hash: &packed::Byte32) -> Option<HeaderView>;

    /// TODO(doc): @quake
    fn block_exists(&self, hash: &packed::Byte32) -> bool;

    /// Gets cellbase by block hash
    fn get_cellbase(&self, hash: &packed::Byte32) -> Option<TransactionView>;

    /// Gets latest built filter data block hash
    fn get_latest_built_filter_data_block_hash(&self) -> Option<packed::Byte32>;

    /// Gets block filter data by block hash
    fn get_block_filter(&self, hash: &packed::Byte32) -> Option<packed::Bytes>;

    /// Gets block filter hash by block hash
    fn get_block_filter_hash(&self, hash: &packed::Byte32) -> Option<packed::Byte32>;

    /// Gets block bytes by block hash
    fn get_packed_block(&self, hash: &packed::Byte32) -> Option<packed::Block>;

    /// Gets block header bytes by block hash
    fn get_packed_block_header(&self, hash: &packed::Byte32) -> Option<packed::Header>;

    /// Gets a header digest.
    fn get_header_digest(&self, position_u64: u64) -> Option<packed::HeaderDigest>;

    /// Gets ancestor block header by a base block hash and number
    fn get_ancestor(&self, base: &packed::Byte32, number: BlockNumber) -> Option<HeaderView>;
}

/// DataLoaderWrapper wrap`ChainStore`
/// impl `HeaderProvider` `CellDataProvider` `EpochProvider`
pub struct DataLoaderWrapper<T>(Arc<T>);

// auto derive don't work
impl<T> Clone for DataLoaderWrapper<T> {
    fn clone(&self) -> Self {
        DataLoaderWrapper(Arc::clone(&self.0))
    }
}

/// Auto transform Arc wrapped `ChainStore` to `DataLoaderWrapper`
pub trait AsDataLoader<T> {
    /// Return arc cloned DataLoaderWrapper
    fn as_data_loader(&self) -> DataLoaderWrapper<T>;
}

impl<T> AsDataLoader<T> for Arc<T>
where
    T: ChainStore,
{
    fn as_data_loader(&self) -> DataLoaderWrapper<T> {
        DataLoaderWrapper(Arc::clone(self))
    }
}

impl<T> CellDataProvider for DataLoaderWrapper<T>
where
    T: ChainStore,
{
    fn get_cell_data(&self, out_point: &OutPoint) -> Option<Bytes> {
        ChainStore::get_cell_data(self.0.as_ref(), out_point).map(|(data, _)| data)
    }

    fn get_cell_data_hash(&self, out_point: &OutPoint) -> Option<Byte32> {
        ChainStore::get_cell_data_hash(self.0.as_ref(), out_point)
    }
}

impl<T> HeaderProvider for DataLoaderWrapper<T>
where
    T: ChainStore,
{
    fn get_header(&self, block_hash: &Byte32) -> Option<HeaderView> {
        ChainStore::get_block_header(self.0.as_ref(), block_hash)
    }
}

impl<T> HeaderFieldsProvider for DataLoaderWrapper<T>
where
    T: ChainStore,
{
    fn get_header_fields(&self, hash: &Byte32) -> Option<HeaderFields> {
        self.0.get_block_header(hash).map(|header| HeaderFields {
            number: header.number(),
            epoch: header.epoch(),
            parent_hash: header.data().raw().parent_hash(),
            timestamp: header.timestamp(),
            hash: header.hash(),
        })
    }
}

impl<T> EpochProvider for DataLoaderWrapper<T>
where
    T: ChainStore,
{
    fn get_epoch_ext(&self, header: &HeaderView) -> Option<EpochExt> {
        ChainStore::get_block_epoch_index(self.0.as_ref(), &header.hash())
            .and_then(|index| ChainStore::get_epoch_ext(self.0.as_ref(), &index))
    }

    fn get_block_hash(&self, number: BlockNumber) -> Option<Byte32> {
        ChainStore::get_block_hash(self.0.as_ref(), number)
    }

    fn get_block_ext(&self, block_hash: &Byte32) -> Option<BlockExt> {
        ChainStore::get_block_ext(self.0.as_ref(), block_hash)
    }

    fn get_block_header(&self, hash: &Byte32) -> Option<HeaderView> {
        ChainStore::get_block_header(self.0.as_ref(), hash)
    }
}

impl<T> ExtensionProvider for DataLoaderWrapper<T>
where
    T: ChainStore,
{
    fn get_block_extension(&self, hash: &Byte32) -> Option<packed::Bytes> {
        ChainStore::get_block_extension(self.0.as_ref(), hash)
    }
}

/// Borrowed DataLoaderWrapper with lifetime
pub struct BorrowedDataLoaderWrapper<'a, T>(&'a T);
impl<'a, T: ChainStore> BorrowedDataLoaderWrapper<'a, T> {
    /// Construct new BorrowedDataLoaderWrapper
    pub fn new(source: &'a T) -> Self {
        BorrowedDataLoaderWrapper(source)
    }
}

impl<'a, T: ChainStore> CellDataProvider for BorrowedDataLoaderWrapper<'a, T> {
    fn get_cell_data(&self, out_point: &OutPoint) -> Option<Bytes> {
        self.0.get_cell_data(out_point).map(|(data, _)| data)
    }

    fn get_cell_data_hash(&self, out_point: &OutPoint) -> Option<Byte32> {
        self.0.get_cell_data_hash(out_point)
    }
}

impl<'a, T: ChainStore> HeaderProvider for BorrowedDataLoaderWrapper<'a, T> {
    fn get_header(&self, block_hash: &Byte32) -> Option<HeaderView> {
        self.0.get_block_header(block_hash)
    }
}

impl<'a, T: ChainStore> EpochProvider for BorrowedDataLoaderWrapper<'a, T> {
    fn get_epoch_ext(&self, header: &HeaderView) -> Option<EpochExt> {
        ChainStore::get_block_epoch_index(self.0, &header.hash())
            .and_then(|index| ChainStore::get_epoch_ext(self.0, &index))
    }

    fn get_block_hash(&self, number: BlockNumber) -> Option<Byte32> {
        ChainStore::get_block_hash(self.0, number)
    }

    fn get_block_ext(&self, block_hash: &Byte32) -> Option<BlockExt> {
        ChainStore::get_block_ext(self.0, block_hash)
    }

    fn get_block_header(&self, hash: &Byte32) -> Option<HeaderView> {
        ChainStore::get_block_header(self.0, hash)
    }
}

impl<'a, T: ChainStore> ExtensionProvider for BorrowedDataLoaderWrapper<'a, T> {
    fn get_block_extension(&self, hash: &Byte32) -> Option<packed::Bytes> {
        ChainStore::get_block_extension(self.0, hash)
    }
}
