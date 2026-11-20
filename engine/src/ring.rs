//! Single-producer, single-consumer audio queue without locks or allocations.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub struct AudioRing {
    slots: Box<[AtomicU32]>,
    read: AtomicUsize,
    write: AtomicUsize,
    overruns: AtomicU64,
    underruns: AtomicU64,
}

impl AudioRing {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 1);
        Self {
            slots: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
            overruns: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
        }
    }

    pub fn push(&self, sample: f32) {
        let write = self.write.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        if write.wrapping_sub(read) >= self.slots.len() {
            self.overruns.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.slots[write % self.slots.len()].store(sample.to_bits(), Ordering::Relaxed);
        self.write.store(write.wrapping_add(1), Ordering::Release);
    }

    pub fn pop(&self) -> f32 {
        let read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        if read == write {
            self.underruns.fetch_add(1, Ordering::Relaxed);
            return 0.0;
        }
        let bits = self.slots[read % self.slots.len()].load(Ordering::Relaxed);
        self.read.store(read.wrapping_add(1), Ordering::Release);
        f32::from_bits(bits)
    }

    /// On activation, discard stale microphone audio accumulated while no
    /// application was recording from the virtual source.
    pub fn trim_to(&self, maximum_frames: usize) {
        let read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        if write.wrapping_sub(read) > maximum_frames {
            self.read
                .store(write.wrapping_sub(maximum_frames), Ordering::Release);
        }
    }

    pub fn overruns(&self) -> u64 {
        self.overruns.load(Ordering::Relaxed)
    }

    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_float_bits_and_counts_underrun() {
        let ring = AudioRing::new(4);
        ring.push(-0.75);
        assert_eq!(ring.pop().to_bits(), (-0.75_f32).to_bits());
        assert_eq!(ring.pop(), 0.0);
        assert_eq!(ring.underruns(), 1);
    }

    #[test]
    fn full_queue_drops_newest_sample() {
        let ring = AudioRing::new(2);
        ring.push(1.0);
        ring.push(2.0);
        ring.push(3.0);
        assert_eq!((ring.pop(), ring.pop()), (1.0, 2.0));
        assert_eq!(ring.overruns(), 1);
    }

    #[test]
    fn trim_discards_stale_samples() {
        let ring = AudioRing::new(8);
        for sample in 1..=6 {
            ring.push(sample as f32);
        }
        ring.trim_to(2);
        assert_eq!((ring.pop(), ring.pop()), (5.0, 6.0));
    }
}
