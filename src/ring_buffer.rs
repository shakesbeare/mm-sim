use core::mem::MaybeUninit;

use bevy::platform::prelude::Vec;

pub struct RingBuffer<T> {
    /// The number of elements the buffer can hold
    capacity: usize,
    /// The current number of elements stored in the buffer
    length: usize,
    /// The current start of the buffer
    head: usize,
    /// The current end of the buffer
    tail: usize,
    /// The data on the heap
    buf: Box<[T]>,
}

impl<T: std::fmt::Debug> std::fmt::Debug for RingBuffer<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Clone> Clone for RingBuffer<T> {
    fn clone(&self) -> Self {
        Self {
            capacity: self.capacity,
            length: self.length,
            head: self.head,
            tail: self.tail,
            buf: self.buf.clone(),
        }
    }
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        let mut vec = Vec::with_capacity(capacity);
        // SAFETY:
        //     Elements will not be able to be accessed until they are initialized
        unsafe { vec.resize_with(capacity, || std::mem::zeroed()) };
        let buf: Box<[T]> = vec.into_boxed_slice();
        Self {
            capacity,
            length: 0,
            head: 0,
            tail: 0,
            buf,
        }
    }

    /// Converts the index into the internal representation to retrieve the correct data
    #[inline]
    fn index_conv(&self, index: usize) -> Option<usize> {
        if index > self.len() {
            return None;
        }
        let out = (self.head + index) % self.capacity;

        Some(out)
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        let index = self.index_conv(index)?;
        self.buf.get(index)
    }

    #[inline]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        let index = self.index_conv(index)?;
        self.buf.get_mut(index)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.length
    }

    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    fn inc_head(&mut self) {
        self.head = (self.head + 1) % self.capacity;
    }

    #[inline]
    fn inc_tail(&mut self) {
        let tail = self.tail + 1;
        if tail > self.capacity {
            self.tail = tail - self.capacity;
        } else {
            self.tail = tail;
        }
    }

    #[inline]
    fn dec_tail(&mut self) {
        if self.tail == 0 {
            self.head = self.capacity - 1;
        } else {
            self.head -= 1;
        }
    }

    #[inline]
    pub fn push(&mut self, value: T) {
        let tail = if self.tail == self.capacity {
            0
        } else {
            self.tail
        };
        self.buf[tail] = value;
        self.inc_tail();
        if self.length < self.capacity {
            self.length += 1;
        } else {
            self.inc_head();
        }
    }

    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        if self.length == 0 {
            return None;
        }
        let tail = self.tail;
        // SAFETY:
        //     tail is decremented which prevents access to de-initialized memory
        let mut taken: T = unsafe { std::mem::zeroed() };
        std::mem::swap(&mut self[tail], &mut taken);
        self.dec_tail();
        self.length -= 1;
        Some(taken)
    }

    pub fn iter<'a>(&'a self) -> RingBufferFiniteIter<'a, T> {
        RingBufferFiniteIter { buf: self, cur: 0 }
    }

    pub fn iter_mut<'a>(&'a mut self) -> RingBufferFiniteIterMut<'a, T> {
        RingBufferFiniteIterMut { buf: self, cur: 0 }
    }
}

impl<T> std::ops::Index<usize> for RingBuffer<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        let index = self.index_conv(index).expect("Index out of bounds");
        &self.buf[index]
    }
}

impl<T> std::ops::IndexMut<usize> for RingBuffer<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let index = self.index_conv(index).expect("Index out of bounds");
        &mut self.buf[index]
    }
}

impl<T> std::iter::IntoIterator for RingBuffer<T> {
    type Item = T;
    type IntoIter = RingBufferFiniteIntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        RingBufferFiniteIntoIter {
            buf: unsafe { std::mem::transmute::<RingBuffer<T>, RingBuffer<MaybeUninit<T>>>(self) },
            cur: 0,
        }
    }
}

pub struct RingBufferFiniteIterMut<'a, T> {
    buf: &'a mut RingBuffer<T>,
    cur: usize,
}

impl<'a, T> Iterator for RingBufferFiniteIterMut<'a, T> {
    type Item = &'a mut T;

    #[inline]
    fn next<'s>(&'s mut self) -> Option<Self::Item> {
        if self.cur < self.buf.len() {
            // SAFETY:
            //     The mutable reference may live as long as the structure itself
            //     So long as only one mutable reference may ever be obtained from the iteration
            //     process
            //     As long as self.cur never goes backwards, it is impossible for the same element
            //     to be returned multiple times
            let item: &'a mut T = unsafe { std::mem::transmute(self.buf.get_mut(self.cur)?) };
            self.cur += 1;
            return Some(item);
        } else {
            None
        }
    }
}

pub struct RingBufferFiniteIter<'a, T> {
    buf: &'a RingBuffer<T>,
    cur: usize,
}

impl<'a, T> Iterator for RingBufferFiniteIter<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.cur < self.buf.len() {
            let item = self.buf.get(self.cur)?;
            self.cur += 1;
            return Some(item);
        } else {
            None
        }
    }
}

pub struct RingBufferFiniteIntoIter<T> {
    buf: RingBuffer<MaybeUninit<T>>,
    cur: usize,
}

impl<T> Iterator for RingBufferFiniteIntoIter<T> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.cur < self.buf.len() {
            let item = self.buf.get_mut(self.cur)?;
            let mut taken: MaybeUninit<T> = MaybeUninit::uninit();
            std::mem::swap(item, &mut taken);
            // SAFETY:
            //     self.cur must never decrease, ensuring the the now-uninitialized memory is never
            //     read
            let taken = unsafe { taken.assume_init() };
            self.cur += 1;

            Some(taken)
        } else {
            None
        }
    }
}
