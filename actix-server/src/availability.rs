use crate::worker::WorkerHandleAccept;

/// Array of u128 with every bit as marker for a worker handle's availability.
#[derive(Debug, Default)]
pub(crate) struct Availability([u128; 4]);

impl Availability {
    /// Check if any worker handle is available
    #[inline(always)]
    pub(crate) fn available(&self) -> bool {
        self.0.iter().any(|a| *a != 0)
    }

    /// Check if worker handle is available by index
    #[inline(always)]
    pub(crate) fn get_available(&self, idx: usize) -> bool {
        let (offset, idx) = Self::offset(idx);

        self.0[offset] & (1 << idx as u128) != 0
    }

    /// Set worker handle available state by index.
    pub(crate) fn set_available(&mut self, idx: usize, avail: bool) {
        let (offset, idx) = Self::offset(idx);

        let off = 1 << idx as u128;
        if avail {
            self.0[offset] |= off;
        } else {
            self.0[offset] &= !off
        }
    }

    /// Set all worker handle to available state.
    /// This would result in a re-check on all workers' availability.
    pub(crate) fn set_available_all(&mut self, handles: &[WorkerHandleAccept]) {
        handles.iter().for_each(|handle| {
            self.set_available(handle.idx(), true);
        })
    }

    /// Get offset and adjusted index of given worker handle index.
    pub(crate) fn offset(idx: usize) -> (usize, usize) {
        if idx < 128 {
            (0, idx)
        } else if idx < 128 * 2 {
            (1, idx - 128)
        } else if idx < 128 * 3 {
            (2, idx - 128 * 2)
        } else if idx < 128 * 4 {
            (3, idx - 128 * 3)
        } else {
            panic!("Max WorkerHandle count is 512")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(aval: &mut Availability, idx: usize) {
        aval.set_available(idx, true);
        assert!(aval.available());

        aval.set_available(idx, true);

        aval.set_available(idx, false);
        assert!(!aval.available());

        aval.set_available(idx, false);
        assert!(!aval.available());
    }

    fn multi(aval: &mut Availability, mut idx: Vec<usize>) {
        idx.iter().for_each(|idx| aval.set_available(*idx, true));

        assert!(aval.available());

        while let Some(idx) = idx.pop() {
            assert!(aval.available());
            aval.set_available(idx, false);
        }

        assert!(!aval.available());
    }

    #[test]
    fn availability() {
        let mut aval = Availability::default();

        single(&mut aval, 1);
        single(&mut aval, 128);
        single(&mut aval, 256);
        single(&mut aval, 511);

        let idx = (0..511).filter(|i| i % 3 == 0 && i % 5 == 0).collect();

        multi(&mut aval, idx);

        multi(&mut aval, (0..511).collect())
    }

    #[test]
    #[should_panic]
    fn overflow() {
        let mut aval = Availability::default();
        single(&mut aval, 512);
    }

    #[test]
    fn pin_point() {
        let mut aval = Availability::default();

        aval.set_available(438, true);

        aval.set_available(479, true);

        assert_eq!(aval.0[3], 1 << (438 - 384) | 1 << (479 - 384));
    }
}
