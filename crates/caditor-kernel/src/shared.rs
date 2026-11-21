use std::sync::Arc;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct SharedBuffer {
    address: usize,
    bytes: usize,
}

impl SharedBuffer {
    pub fn of<T: ?Sized>(shared: &Arc<T>, bytes: usize) -> Self {
        Self {
            address: Arc::as_ptr(shared).cast::<u8>().addr(),
            bytes,
        }
    }

    pub fn slice<T>(shared: &Arc<[T]>) -> Self {
        Self::of(shared, size_of_val(&**shared))
    }

    pub fn address(self) -> usize {
        self.address
    }

    pub fn bytes(self) -> usize {
        self.bytes
    }
}
