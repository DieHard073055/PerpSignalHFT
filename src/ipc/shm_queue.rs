use memmap2::{MmapMut, MmapOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{fs::OpenOptions, io, ptr};

const HEADER_SIZE: usize = 4096;

#[repr(C)]
struct QueueHeader {
    capacity: u32,   // buffer size in bytes
    head: AtomicU32, // read cursor
    tail: AtomicU32, // write cursor
    _pad: [u8; HEADER_SIZE - 12],
}

pub struct ShmQueue {
    mmap: MmapMut,
    header: *mut QueueHeader,
    buf_off: usize,
    capacity: u32,
}

impl ShmQueue {
    /// Create or open an SPSC queue in /dev/shm with given name and capacity
    pub fn create(name: &str, capacity: u32) -> io::Result<Self> {
        let path = format!("/dev/shm/{}", name);
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&path)?;
        // ensure file is large enough for header + buffer
        let total_size = HEADER_SIZE + capacity as usize;
        file.set_len(total_size as u64)?;

        // memory-map it
        let mut mmap = unsafe { MmapOptions::new().len(total_size).map_mut(&file)? };
        let header_ptr = mmap.as_mut_ptr() as *mut QueueHeader;

        // initialize header if first time (capacity=0 means uninit)
        unsafe {
            if (*header_ptr).capacity == 0 {
                (*header_ptr).capacity = capacity;
                (*header_ptr).head = AtomicU32::new(0);
                (*header_ptr).tail = AtomicU32::new(0);
            }
        }

        Ok(Self {
            mmap,
            header: header_ptr,
            buf_off: HEADER_SIZE,
            capacity,
        })
    }

    /// Push a message (length-prefixed) into the queue
    pub fn push(&self, data: &[u8]) -> io::Result<()> {
        let cap = self.capacity;
        let header = unsafe { &*self.header };
        let tail = header.tail.load(Ordering::Relaxed);
        let head = header.head.load(Ordering::Acquire);
        let free = cap + head - tail;
        let needed = 4 + data.len() as u32;
        if needed > free {
            return Err(io::Error::new(io::ErrorKind::Other, "Queue full"));
        }
        // write length prefix
        self.write_at(tail & (cap - 1), &(data.len() as u32).to_le_bytes());
        // write payload
        self.write_at((tail & (cap - 1)) + 4, data);
        // advance tail
        header.tail.store(tail + needed, Ordering::Release);
        Ok(())
    }

    /// Pop a message, returning None if empty
    pub fn pop(&self) -> io::Result<Option<Vec<u8>>> {
        let cap = self.capacity;
        let header = unsafe { &*self.header };
        let head = header.head.load(Ordering::Relaxed);
        let tail = header.tail.load(Ordering::Acquire);
        if head == tail {
            return Ok(None);
        }
        // read length prefix
        let mut len_buf = [0u8; 4];
        self.read_at(head & (cap - 1), &mut len_buf);
        let len = u32::from_le_bytes(len_buf) as usize;
        // read payload
        let mut data = vec![0u8; len];
        self.read_at((head & (cap - 1)) + 4, &mut data);
        // advance head
        header.head.store(head + 4 + len as u32, Ordering::Release);
        Ok(Some(data))
    }

    /// write bytes at offset (may wrap)
    fn write_at(&self, offset: u32, bytes: &[u8]) {
        let cap = self.capacity as usize;
        let off = offset as usize % cap;
        let end = off + bytes.len();
        // get a mutable pointer from shared mmap without requiring &mut
        let base_ptr = unsafe { self.mmap.as_ptr().add(self.buf_off) };
        if end <= cap {
            let dst = (base_ptr as *mut u8).wrapping_add(off);
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
            }
        } else {
            let first = cap - off;
            let dst1 = (base_ptr as *mut u8).wrapping_add(off);
            let dst2 = base_ptr as *mut u8;
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), dst1, first);
                ptr::copy_nonoverlapping(bytes.as_ptr().add(first), dst2, bytes.len() - first);
            }
        }
    }

    /// read bytes at offset (may wrap)
    fn read_at(&self, offset: u32, dest: &mut [u8]) {
        let cap = self.capacity as usize;
        let off = offset as usize % cap;
        let end = off + dest.len();
        let base_ptr = unsafe { self.mmap.as_ptr().add(self.buf_off) };
        if end <= cap {
            let src = base_ptr.wrapping_add(off);
            unsafe {
                ptr::copy_nonoverlapping(src, dest.as_mut_ptr(), dest.len());
            }
        } else {
            let first = cap - off;
            let src1 = base_ptr.wrapping_add(off);
            let src2 = base_ptr;
            unsafe {
                ptr::copy_nonoverlapping(src1, dest.as_mut_ptr(), first);
                ptr::copy_nonoverlapping(
                    src2,
                    dest.as_mut_ptr().wrapping_add(first),
                    dest.len() - first,
                );
            }
        }
    }
}

// SAFETY: ShmQueue only contains an mmap and a raw pointer into that mmap, which is safe to send
// across threads as long as both sides agree on the shared memory region.
unsafe impl Send for ShmQueue {}
// Multiple readers/writers coordinate via atomics, so Sync is also safe.
unsafe impl Sync for ShmQueue {}
