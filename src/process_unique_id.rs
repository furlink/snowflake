// Copyright 2016 Steven Allen
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
use std::cell::UnsafeCell;

use std::default::Default;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

static GLOBAL_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn next_global() -> usize {
    let mut prev = GLOBAL_COUNTER.load(Ordering::Relaxed);
    loop {
        assert!(
            prev < usize::MAX,
            "Snow Crash: Go home and reevaluate your threading model!"
        );

        let old_value = match GLOBAL_COUNTER.compare_exchange(
            prev,
            prev + 1,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(value) => value,
            Err(value) => value,
        };

        if old_value == prev {
            return prev;
        } else {
            prev = old_value;
        }
    }
}

// NOTE: We could use a Cell (not unsafe) but this is slightly faster.
thread_local! {
    static NEXT_LOCAL_UNIQUE_ID: UnsafeCell<ProcessUniqueId> = UnsafeCell::new(ProcessUniqueId {
        prefix: next_global(),
        offset: 0
    })
}

/// Process unique IDs are guaranteed to be unique within the current process, for the lifetime of
/// the current process.
///
/// 1. ID creation should be highly performant even on highly concurrent systems. It's MUCH faster
///    than using random/time based IDs (but, on the other hand, only unique within a process).
/// 2. While this crate can run out of process unique IDs, this is very unlikely assuming a sane
///    threading model and will panic rather than potentially reusing unique IDs.
///
/// # Limits
///
/// The unique ID's are `sizeof(usize) + 64` bits wide and are generated by combining a `usize`
/// global counter value with a 64bit thread local offset. This is important because each thread
/// that calls `new()` at least once will reserve at least 2^64 IDs. So, the only way to run out of
/// IDs in a reasonable amount of time is to run a 32bit system, spawn 2^32 threads, and claim one
/// ID on each thread. You might be able to do this on a 64bit system but it would take a while...
/// TL; DR: Don't create unique IDs from over 4 billion different threads on a 32bit system.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(feature = "serde_support", derive(Serialize, Deserialize))]
pub struct ProcessUniqueId {
    prefix: usize,
    offset: u64,
}

impl fmt::Display for ProcessUniqueId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "puid-{:x}-{:x}", self.prefix, self.offset)
    }
}

impl ProcessUniqueId {
    /// Create a new unique ID.
    ///
    /// **panics** if there are no more unique IDs available. If this happens, go home and
    /// reevaluate your threading model!
    #[inline]
    pub fn new() -> Self {
        NEXT_LOCAL_UNIQUE_ID.with(|unique_id| {
            unsafe {
                // NOTE: Checked ops are slower than manually checking... (WTF?)
                let next_unique_id = *unique_id.get();
                (*unique_id.get()) = if next_unique_id.offset == u64::MAX {
                    ProcessUniqueId {
                        prefix: next_global(),
                        offset: 0,
                    }
                } else {
                    ProcessUniqueId {
                        prefix: next_unique_id.prefix,
                        offset: next_unique_id.offset + 1,
                    }
                };
                next_unique_id
            }
        })
    }
}

impl Default for ProcessUniqueId {
    #[inline]
    fn default() -> Self {
        ProcessUniqueId::new()
    }
}

#[cfg(test)]
mod test {
    use super::ProcessUniqueId;
    use std::thread;

    // Glass box tests.

    #[test]
    fn test_unique_id_unthreaded() {
        let first_unique_id = ProcessUniqueId::new();
        // Not going to be able to count to u64::MAX
        {
            // Ignore....
            use super::NEXT_LOCAL_UNIQUE_ID;
            NEXT_LOCAL_UNIQUE_ID
                .with(|unique_id| unsafe { (*unique_id.get()).offset = u64::MAX - 10 });
        } // Ignore...

        for i in (u64::MAX - 11)..(u64::MAX) {
            assert!(
                ProcessUniqueId::new()
                    == ProcessUniqueId {
                        prefix: first_unique_id.prefix,
                        offset: i + 1,
                    }
            );
        }
        let next = ProcessUniqueId::new();
        assert!(next.prefix != first_unique_id.prefix);
        assert!(next.offset == 0);
        assert!(
            ProcessUniqueId::new()
                == ProcessUniqueId {
                    prefix: next.prefix,
                    offset: 1,
                }
        );
    }

    #[test]
    fn test_unique_id_threaded() {
        let threads: Vec<_> = (0..128)
            .map(|_| {
                thread::spawn(move || {
                    thread::park();
                    let unique_id = ProcessUniqueId::new();
                    assert_eq!(unique_id.offset, 0);
                    unique_id.prefix
                })
            })
            .collect();

        // Start them all at once.
        for thread in &threads {
            thread.thread().unpark();
        }

        let mut results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        results.sort();
        let old_len = results.len();
        results.dedup();
        assert_eq!(old_len, results.len());
    }

    // #[bench]
    // fn bench_next_global(b: &mut Bencher) {
    //     b.iter(|| {
    //         next_global();
    //     });
    // }

    // #[bench]
    // fn bench_next_global_threaded(b: &mut Bencher) {
    //     let pool = ThreadPool::new(4usize);
    //     b.iter(|| {
    //         let (tx, rx) = channel();
    //         for _ in 0..4 {
    //             let tx = tx.clone();
    //             pool.execute(move || {
    //                 for _ in 0..1000 {
    //                     next_global();
    //                 }
    //                 tx.send(()).unwrap();
    //             });
    //         }
    //         rx.iter().take(4).count();
    //     });
    // }

    // #[bench]
    // fn bench_unique_id(b: &mut Bencher) {
    //     b.iter(|| {
    //         ProcessUniqueId::new();
    //     });
    // }

    // #[bench]
    // fn bench_random_id(b: &mut Bencher) {
    //     use self::rand::random;
    //     b.iter(|| {
    //         let _: u64 = random();
    //     });
    // }

    // #[bench]
    // fn bench_time_id(b: &mut Bencher) {
    //     use self::time::get_time;
    //     b.iter(|| {
    //         let _ = get_time();
    //     });
    // }

    // #[bench]
    // fn bench_uuid(b: &mut Bencher) {
    //     use self::uuid::Uuid;
    //     b.iter(|| {
    //         Uuid::new_v4();
    //     });
    // }

    // #[bench]
    // fn bench_unique_id_threaded(b: &mut Bencher) {
    //     let pool = ThreadPool::new(4usize);
    //     b.iter(|| {
    //         let (tx, rx) = channel();
    //         for _ in 0..4 {
    //             let tx = tx.clone();
    //             pool.execute(move || {
    //                 for _ in 0..1000 {
    //                     ProcessUniqueId::new();
    //                 }
    //                 tx.send(()).unwrap();
    //             });
    //         }
    //         rx.iter().take(4).count();
    //     });
    // }
}
