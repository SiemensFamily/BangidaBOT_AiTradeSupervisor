/// A fixed-size ring buffer backed by a pre-allocated `Vec`.
///
/// Circular writes overwrite the oldest element when full.
/// No heap allocation occurs after construction.
#[derive(Debug, Clone)]
pub struct RingBuffer<T> {
    buf: Vec<T>,
    capacity: usize,
    /// Points to the next write position.
    head: usize,
    /// Number of elements currently stored.
    len: usize,
}

impl<T: Clone + Default> RingBuffer<T> {
    /// Create a new ring buffer with the given capacity, pre-allocated.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "RingBuffer capacity must be > 0");
        let mut buf = Vec::with_capacity(capacity);
        buf.resize(capacity, T::default());
        Self {
            buf,
            capacity,
            head: 0,
            len: 0,
        }
    }

    /// Push an item into the buffer, overwriting the oldest if full.
    #[inline]
    pub fn push(&mut self, item: T) {
        self.buf[self.head] = item;
        self.head = (self.head + 1) % self.capacity;
        if self.len < self.capacity {
            self.len += 1;
        }
    }

    /// Number of elements currently stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer contains zero elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the buffer is at capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.len == self.capacity
    }

    /// The maximum number of elements the buffer can hold.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the most recently pushed element.
    #[inline]
    pub fn last(&self) -> Option<&T> {
        if self.len == 0 {
            return None;
        }
        let idx = (self.head + self.capacity - 1) % self.capacity;
        Some(&self.buf[idx])
    }

    /// Get element by logical index (0 = oldest, len-1 = newest).
    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        let start = if self.len < self.capacity {
            0
        } else {
            self.head
        };
        let actual = (start + index) % self.capacity;
        Some(&self.buf[actual])
    }

    /// Iterate from oldest to newest.
    pub fn iter(&self) -> RingBufferIter<'_, T> {
        RingBufferIter {
            buf: self,
            pos: 0,
        }
    }
}

pub struct RingBufferIter<'a, T> {
    buf: &'a RingBuffer<T>,
    pos: usize,
}

impl<'a, T: Clone + Default> Iterator for RingBufferIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.buf.len {
            return None;
        }
        let item = self.buf.get(self.pos);
        self.pos += 1;
        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.buf.len - self.pos;
        (remaining, Some(remaining))
    }
}

impl<'a, T: Clone + Default> ExactSizeIterator for RingBufferIter<'a, T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_push_and_iter() {
        let mut rb = RingBuffer::new(3);
        rb.push(1);
        rb.push(2);
        rb.push(3);
        assert!(rb.is_full());
        let v: Vec<_> = rb.iter().copied().collect();
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn overwrites_oldest() {
        let mut rb = RingBuffer::new(3);
        rb.push(1);
        rb.push(2);
        rb.push(3);
        rb.push(4);
        assert_eq!(rb.len(), 3);
        let v: Vec<_> = rb.iter().copied().collect();
        assert_eq!(v, vec![2, 3, 4]);
        assert_eq!(rb.last(), Some(&4));
    }

    #[test]
    fn get_by_index() {
        let mut rb = RingBuffer::new(3);
        rb.push(10);
        rb.push(20);
        assert_eq!(rb.get(0), Some(&10));
        assert_eq!(rb.get(1), Some(&20));
        assert_eq!(rb.get(2), None);
    }
}
