//! Eviction grace for the image caches.
//!
//! An entry that is drawn one frame and not the next is usually about to
//! come back: a cover scrolling past the edge of a shelf, an icon behind a
//! page transition. Evicting it on that first idle frame means decoding and
//! uploading a multi-megabyte texture again when it returns, and doing so
//! for every cover along a scroll. The caches instead count the consecutive
//! trim passes in which an entry went unused and drop it only once that
//! streak has outrun the grace.

use rustc_hash::FxHashMap;

use std::hash::Hash;

/// Trim passes an entry may go unused before it is evicted.
///
/// A trim runs at most once per presented frame, and only on frames that
/// uploaded something new, so this is a floor of one second at 60 fps and
/// stretches while nothing is being loaded. Memory stays bounded either
/// way: an idle entry only outlives its grace while no upload lands, and
/// nothing is added to the cache without one.
pub const GRACE: u32 = 60;

/// Consecutive unused trim passes, per entry that is currently idle.
///
/// Entries in use are absent: the map only grows with what is going stale,
/// and shrinks again as soon as an entry is drawn or dropped. It holds no
/// key its cache does not, because every trim visits every cached entry and
/// either touches or expires it.
#[derive(Debug)]
pub struct Idle<K> {
    passes: FxHashMap<K, u32>,
}

impl<K> Default for Idle<K> {
    fn default() -> Self {
        Self {
            passes: FxHashMap::default(),
        }
    }
}

impl<K: Hash + Eq + Copy> Idle<K> {
    /// Records that `key` was used, ending its idle streak.
    pub fn touch(&mut self, key: &K) {
        let _ = self.passes.remove(key);
    }

    /// Counts one unused pass for `key`.
    ///
    /// Returns `true` once the streak has outrun [`GRACE`]; the entry is then
    /// forgotten here too, since the caller is about to drop it.
    pub fn expired(&mut self, key: &K) -> bool {
        let passes = self.passes.entry(*key).or_insert(0);
        *passes += 1;

        if *passes > GRACE {
            let _ = self.passes.remove(key);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_survives_the_grace_and_expires_after_it() {
        let mut idle = Idle::default();

        for _ in 0..GRACE {
            assert!(!idle.expired(&1u32));
        }

        assert!(idle.expired(&1u32));
        assert!(idle.passes.is_empty(), "an expired entry is forgotten");
    }

    #[test]
    fn a_touch_resets_the_streak() {
        let mut idle = Idle::default();

        for _ in 0..GRACE {
            assert!(!idle.expired(&1u32));
        }

        idle.touch(&1u32);

        assert!(!idle.expired(&1u32));
        assert_eq!(idle.passes.get(&1u32), Some(&1));
    }

    #[test]
    fn a_touch_of_an_entry_in_use_is_a_no_op() {
        let mut idle = Idle::<u32>::default();

        idle.touch(&7);

        assert!(idle.passes.is_empty(), "entries in use are never tracked");
    }

    #[test]
    fn streaks_are_counted_per_entry() {
        let mut idle = Idle::default();

        for _ in 0..GRACE {
            assert!(!idle.expired(&1u32));
        }

        // A neighbour that only just went idle is untouched by the first
        // one expiring, and the first one expiring leaves it alone.
        assert!(!idle.expired(&2u32));
        assert!(idle.expired(&1u32));
        assert_eq!(idle.passes.len(), 1);
        assert_eq!(idle.passes.get(&2u32), Some(&1));
    }

    #[test]
    fn an_expired_entry_that_returns_gets_a_full_grace_again() {
        let mut idle = Idle::default();

        for _ in 0..GRACE {
            assert!(!idle.expired(&1u32));
        }
        assert!(idle.expired(&1u32));

        // The cache re-inserts the same key after a reload: the old streak
        // must not carry over and evict it on its first idle pass.
        for _ in 0..GRACE {
            assert!(!idle.expired(&1u32));
        }
        assert!(idle.expired(&1u32));
    }

    #[test]
    fn a_touch_and_an_expiry_never_leave_a_stale_key_behind() {
        let mut idle = Idle::default();

        for key in 0..8u32 {
            assert!(!idle.expired(&key));
        }
        for key in 0..8u32 {
            if key % 2 == 0 {
                idle.touch(&key);
            }
        }

        assert_eq!(idle.passes.len(), 4);

        for _ in 0..GRACE {
            for key in (1..8u32).step_by(2) {
                let _ = idle.expired(&key);
            }
        }

        assert!(idle.passes.is_empty(), "everything idle has expired");
    }
}
