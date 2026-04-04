/// A fixed-capacity circular buffer for time-series data.
#[derive(Debug, Clone)]
pub struct RingBuffer<T> {
    data: Vec<T>,
    capacity: usize,
    head: usize,
    len: usize,
}

impl<T: Clone> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "RingBuffer capacity must be > 0");
        Self {
            data: Vec::with_capacity(capacity),
            capacity,
            head: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, value: T) {
        if self.data.len() < self.capacity {
            // Still filling the initial vec
            self.data.push(value);
            self.head = self.data.len() % self.capacity;
            self.len = self.data.len();
        } else {
            // Overwrite at head position
            self.data[self.head] = value;
            self.head = (self.head + 1) % self.capacity;
            self.len = self.capacity;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_full(&self) -> bool {
        self.len == self.capacity
    }

    /// Iterate from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        let start = if self.len < self.capacity {
            0
        } else {
            self.head
        };
        let len = self.len;
        let cap = self.capacity;
        let data = &self.data;
        (0..len).map(move |i| &data[(start + i) % cap])
    }

    /// Most recently pushed element.
    pub fn latest(&self) -> Option<&T> {
        if self.is_empty() {
            return None;
        }
        let idx = if self.len < self.capacity {
            self.len - 1
        } else {
            (self.head + self.capacity - 1) % self.capacity
        };
        Some(&self.data[idx])
    }

    /// Oldest element still in the buffer.
    pub fn oldest(&self) -> Option<&T> {
        if self.is_empty() {
            return None;
        }
        if self.len < self.capacity {
            Some(&self.data[0])
        } else {
            Some(&self.data[self.head])
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_len() {
        let mut rb = RingBuffer::new(3);
        assert!(rb.is_empty());
        rb.push(1);
        rb.push(2);
        assert_eq!(rb.len(), 2);
        assert!(!rb.is_full());
        rb.push(3);
        assert!(rb.is_full());
        assert_eq!(rb.len(), 3);
    }

    #[test]
    fn test_wrap_around() {
        let mut rb = RingBuffer::new(3);
        rb.push(1);
        rb.push(2);
        rb.push(3);
        rb.push(4); // overwrites 1
        rb.push(5); // overwrites 2
        let items: Vec<_> = rb.iter().cloned().collect();
        assert_eq!(items, vec![3, 4, 5]);
        assert_eq!(*rb.oldest().unwrap(), 3);
        assert_eq!(*rb.latest().unwrap(), 5);
    }

    #[test]
    fn test_iter_order() {
        let mut rb = RingBuffer::new(4);
        for i in 0..7 {
            rb.push(i);
        }
        let items: Vec<_> = rb.iter().cloned().collect();
        assert_eq!(items, vec![3, 4, 5, 6]);
    }

    #[test]
    fn test_latest_oldest_empty() {
        let rb: RingBuffer<i32> = RingBuffer::new(5);
        assert!(rb.latest().is_none());
        assert!(rb.oldest().is_none());
    }
}
