//! Smallest distinct public card IDs with an explicit occupied length.
//! Every u16, including 65535, is a legal ID; no value is a missing-entry sentinel.
#[derive(Clone, Copy)]
pub(super) struct SmallestIds<const N: usize> {
    ids: [u16; N],
    len: u8,
}

impl<const N: usize> SmallestIds<N> {
    pub(super) fn new() -> Self {
        assert!(N > 0 && N <= u8::MAX as usize);
        Self {
            ids: [0; N],
            len: 0,
        }
    }

    #[inline]
    pub(super) fn as_slice(&self) -> &[u16] {
        &self.ids[..usize::from(self.len)]
    }

    /// Retain the N smallest distinct IDs seen so far, in ascending order.
    #[inline]
    pub(super) fn insert(&mut self, id: u16) {
        let pos = self.as_slice().partition_point(|&old| old < id);
        if self.as_slice().get(pos) == Some(&id) || pos == N {
            return;
        }
        let len = (usize::from(self.len) + 1).min(N);
        for index in (pos + 1..len).rev() {
            self.ids[index] = self.ids[index - 1];
        }
        self.ids[pos] = id;
        self.len = len as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_endpoints_are_values_not_padding() {
        let mut ids = SmallestIds::<3>::new();
        assert!(ids.as_slice().is_empty());
        ids.insert(u16::MAX);
        ids.insert(0);
        ids.insert(u16::MAX);
        assert_eq!(ids.as_slice(), &[0, u16::MAX]);
        ids.insert(19);
        assert_eq!(ids.as_slice(), &[0, 19, u16::MAX]);
        ids.insert(5);
        assert_eq!(ids.as_slice(), &[0, 5, 19]);
    }

    #[test]
    fn every_prefix_matches_sorted_distinct_ids() {
        let mut ids = SmallestIds::<5>::new();
        let mut expected = Vec::new();
        for id in [65535, 65534, 3, 0, 3, 65535, 10, 9, 8, 7, 6, 5, 4, 2, 1] {
            ids.insert(id);
            expected.push(id);
            expected.sort_unstable();
            expected.dedup();
            expected.truncate(5);
            assert_eq!(ids.as_slice(), &expected);
        }
    }
}
