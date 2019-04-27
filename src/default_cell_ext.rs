use std::cell::Cell;

pub trait DefaultCellExt<T> {
    fn mutate(&self, f: impl FnOnce(&mut T));
}

impl<T: Default> DefaultCellExt<T> for Cell<T> {
    fn mutate(&self, f: impl FnOnce(&mut T)) {
        let mut value = self.take();
        f(&mut value);
        self.set(value)
    }
}
