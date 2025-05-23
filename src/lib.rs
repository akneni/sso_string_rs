use std::{alloc::{self, Layout}, fmt::{self, Debug}, hash::Hash, hint, mem, slice, str};

#[derive(Debug, Clone, Copy)]
pub enum SsosPrecond {
    Inline,
    InlineAssumeCapacity,
    Static,
    Heap,
    HeapAssumeCapacity,
    Ascii,
}

/// SsoString Precondition
impl SsosPrecond {
    const fn from(num: SsosPrecondType) -> Self {
        match num {
            0 => Self::Inline,
            1 => Self::InlineAssumeCapacity,
            2 => Self::Static,
            3 => Self::Heap,
            4 => Self::HeapAssumeCapacity,
            5 => Self::Ascii,
            _ => panic!("invalid integer"),
        }
    }
    
    pub const fn into_param(&self) -> SsosPrecondType {
        match self {
            Self::Inline => 0,
            Self::InlineAssumeCapacity => 1,
            Self::Static => 2,
            Self::Heap => 3,
            Self::HeapAssumeCapacity => 4,
            Self::Ascii => 5,
        }
    }
}

/// SsoString Precondition Type
pub type SsosPrecondType = u32;




/// Bits 0-4: Length of the string (if inlined)
/// Bit 5: Empty (reserved for future use??)
/// Bit 6: flag IS_STATIC. Any operations requiring mutable state need to copy the underlying data if this bit is set to 1 (this flag is irrelevent for inlined strings). Enables us to do CoW optimizations. 
/// Bit 7: IS_INLINED. 
struct SsoStrMetadata {
    data: u8,
}

pub struct SsoString {
    pointer: *mut u8,
    length: usize,
    capacity: usize,
}

pub trait SsoStringable {
    fn to_sso_string(&self) -> SsoString;
}

impl SsoStrMetadata {
    #[inline]
    fn inline_len(&self) -> u8 {
        self.data & 0b000_11111
    }

    #[inline]
    fn is_static(&self) -> bool {
        (self.data & 0b010_00000) >> 6 == 1
    }

    #[inline]
    fn is_inlined(&self) -> bool {
        self.data >> 7 == 1
    }

    #[inline]
    fn set_inline_len(&mut self, length: u8) {
        self.data = self.data & 0b111_00000;
        self.data = self.data | length;
    }

    #[inline]
    fn set_is_static(&mut self, flag: u8) {
        self.data = self.data & 0b101_11111;
        self.data = self.data | (flag << 6);
    }

    #[inline]
    fn set_is_inlined(&mut self, flag: u8) {
        self.data = self.data & 0b011_11111;
        self.data = self.data | (flag << 7);
    }

    #[inline]
    #[allow(unused)]
    fn zero_all(&mut self) {
        self.data = 0;
    }

    #[inline]
    fn zero_flags(&mut self) {
        self.data = self.data & 0b000_11111;
    }
}


impl SsoString {
    const BIT_MASK_LOWER_24: u32 = 0x00FFFFFF;
    const BIT_MASK_LOWER_56: u64 = 0x00FFFFFFFFFFFFFF;

    const INLINE_CAPACITY: usize = 23;
    const MD_OFFSET: usize = const { mem::size_of::<usize>() - 1 };

    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from(s: impl AsRef<str>) -> Self {
        let s = s.as_ref();
        if s.len() > Self::INLINE_CAPACITY {
            let layout = unsafe { Layout::from_size_align_unchecked(s.len(), 4) };
    
            let string = SsoString { 
                capacity: s.len(), 
                length: s.len(),
                pointer: unsafe { alloc::alloc(layout) } 
            };

            unsafe {
                string.pointer.copy_from_nonoverlapping(s.as_ptr(), s.len());
            }

            return string;
        }

        let mut string = Self::null_string();
        let metadata = string.metadata_mut();
        metadata.set_inline_len(s.len() as u8);
        metadata.set_is_inlined(1);
        
        let ptr = string.inline_ptr_mut();
        unsafe { ptr.copy_from_nonoverlapping(s.as_ptr(), s.len()) };

        string
    }

    #[inline]
    pub fn from_static(s: &'static str) -> Self {
        unsafe { Self::from_static_unchecked(s) }
    }

    #[inline]
    pub unsafe fn from_static_unchecked(s: &str) -> Self {
        let mut string = SsoString { 
            capacity: s.len(),
            length: s.len(),
            pointer: s.as_ptr() as *mut u8,
        };

        let md = string.metadata_mut();
        md.set_is_static(1);

        return string;
    }

    pub fn from_utf8(bytes: Vec<u8>) -> Result<Self, str::Utf8Error> {
        match std::str::from_utf8(&bytes) {
            Ok(_) => {
                let string = unsafe { Self::from_utf8_unchecked(bytes) };
                Ok(string)
            }
            Err(e) => {
                Err(e)
            }
        }
    }
    
    #[inline]
    pub unsafe fn from_utf8_unchecked(bytes: Vec<u8>) -> Self {
        let len = bytes.len();
        let capacity = bytes.capacity();

        let mut md_bytes = mem::ManuallyDrop::new(bytes);

        SsoString {
            pointer: md_bytes.as_mut_ptr(),
            length: len,
            capacity: capacity,
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        let layout = unsafe {
            Layout::from_size_align_unchecked(cap, 4)
        };

        Self {
            length: 0,
            capacity: cap,
            pointer: unsafe { alloc::alloc(layout) }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        if self.is_inlined() {
            self.metadata().inline_len() as usize
        } else {
            self.length
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        if self.is_inlined() {
            return Self::INLINE_CAPACITY;
        }

        match mem::size_of::<usize>() {
            4 => self.capacity & Self::BIT_MASK_LOWER_24 as usize,
            8 => self.capacity & Self::BIT_MASK_LOWER_56 as usize,
            _ => unsafe { hint::unreachable_unchecked() }
        }
    }

    #[inline]
    pub fn is_inlined(&self) -> bool {
        self.metadata().is_inlined()
    }

    #[inline]
    pub fn is_static(&self) -> bool {
        self.metadata().is_static()
    }

    pub fn push_str(&mut self, s: &str) {
        let s_len = s.len();
        let s_ptr = s.as_ptr();
        
        // Cache metadata access - this is expensive due to pointer casting
        let md_data = self.metadata().data;
        let is_inlined = (md_data & 0b100_00000) != 0;
        
        if is_inlined {
            let curr_len = (md_data & 0b000_11111) as usize;
            let new_len = curr_len + s_len;
            
            if new_len <= Self::INLINE_CAPACITY {
                // Fast path: inline to inline
                let dst = unsafe { self.inline_ptr_mut().add(curr_len) };
                unsafe { dst.copy_from_nonoverlapping(s_ptr, s_len) };
                self.set_metadata(0b100_00000 | new_len);
                return;
            }
            
            // Inline to heap transition
            let new_cap = (new_len * 3) >> 1; // Faster than division
            let layout = unsafe { Layout::from_size_align_unchecked(new_cap, 4) };
            let new_ptr = unsafe { alloc::alloc(layout) };
            
            // Copy existing inline data
            let src = self.inline_ptr();
            unsafe { new_ptr.copy_from_nonoverlapping(src, curr_len) };
            // Append new data
            unsafe { new_ptr.add(curr_len).copy_from_nonoverlapping(s_ptr, s_len) };
            
            self.capacity = new_cap;
            self.length = new_len;
            self.pointer = new_ptr;
            return;
        }
        
        // Heap-allocated path
        let is_static = (md_data & 0b010_00000) != 0;
        let curr_len = self.length;
        let new_len = curr_len + s_len;
        
        if is_static {            
            if new_len <= Self::INLINE_CAPACITY {
                // Static to inline
                let dst = self.inline_ptr_mut();
                unsafe { dst.copy_from_nonoverlapping(self.pointer, curr_len) };
                unsafe { dst.add(curr_len).copy_from_nonoverlapping(s_ptr, s_len) };
                self.set_metadata(0b100_00000 | new_len);
                return;
            }
            
            // Static to heap
            let new_cap = (new_len * 3) >> 1;
            let layout = unsafe { Layout::from_size_align_unchecked(new_cap, 4) };
            let new_ptr = unsafe { alloc::alloc(layout) };
            
            unsafe { new_ptr.copy_from_nonoverlapping(self.pointer, curr_len) };
            unsafe { new_ptr.add(curr_len).copy_from_nonoverlapping(s_ptr, s_len) };
            
            self.capacity = new_cap;
            self.length = new_len;
            self.pointer = new_ptr;
            return;
        }
        
        // Mutable heap path
        let curr_cap = self.capacity;
        
        if new_len > curr_cap {
            // Need reallocation
            let new_cap = (new_len * 3) >> 1;
            let new_layout = unsafe { Layout::from_size_align_unchecked(new_cap, 4) };
            
            unsafe {
                self.pointer = alloc::realloc(self.pointer, new_layout, new_cap);
            }
            self.capacity = new_cap;
        }
        
        // Append new data
        unsafe { self.pointer.add(curr_len).copy_from_nonoverlapping(s_ptr, s_len) };
        self.length = new_len;
    }

    /// Assumption is about the current state of the string, not the state after the operation
    /// Ideally, the value passed to `assumption` should be a literal or const value to allow for constant folding
    pub unsafe fn push_str_assume<const PRECOND: SsosPrecondType>(&mut self, s: &str) {
        let s_len = s.len();
        let s_ptr = s.as_ptr();

        let assumption = SsosPrecond::from(PRECOND);
        match assumption {
            SsosPrecond::Inline => {
                let md = self.metadata();
                let length = md.inline_len() as usize;
                let new_length = length + s_len;
                if new_length > Self::INLINE_CAPACITY {
                    self.force_heap_relocation((new_length * 3) >> 1);
                    unsafe {
                        let ptr = self.pointer.add(length);
                        ptr.copy_from_nonoverlapping(s_ptr, s_len);
                        self.length = new_length;
                    }
                }
                else {
                    unsafe {
                        let ptr = self.inline_ptr_mut().add(length);
                        ptr.copy_from_nonoverlapping(s_ptr, s_len);
                        self.metadata_mut().set_inline_len(new_length as u8);
                    }
                }
            }
            SsosPrecond::InlineAssumeCapacity => {
                let md = self.metadata();
                let length = md.inline_len() as usize;
                let new_length = length + s_len;
                unsafe {
                    let ptr = self.inline_ptr_mut().add(length);
                    ptr.copy_from_nonoverlapping(s.as_ptr(), s_len);
                    self.metadata_mut().set_inline_len(new_length as u8);
                }
            }
            SsosPrecond::Static => {
                let new_length = self.length + s_len;
                let new_capacity = (new_length * 3) >> 1;

                unsafe {
                    let layout = Layout::from_size_align_unchecked(new_capacity, 4);
                    let ptr = alloc::alloc(layout);
                    ptr.copy_from_nonoverlapping(self.pointer, self.length);
                    ptr.add(self.length).copy_from_nonoverlapping(s_ptr, s_len);
                    self.pointer = ptr;
                }
                self.capacity = new_capacity;
                self.length = new_length;
            }
            SsosPrecond::Heap => {
                let new_length = self.length + s_len;
                let capacity = self.capacity;
                if new_length > capacity {
                    let new_capacity = (capacity * 3) >> 1;
                    unsafe {
                        let layout = Layout::from_size_align_unchecked(new_capacity, 4);
                        self.pointer = alloc::realloc(self.pointer, layout, new_capacity);
                    }
                    self.capacity = new_capacity;
                }
                unsafe {
                    self.pointer
                        .add(self.length)
                        .copy_from_nonoverlapping(s_ptr, s_len);
                }
                self.length += s_len;
            }
            SsosPrecond::HeapAssumeCapacity => {
                unsafe {
                    self.pointer
                        .add(self.length)
                        .copy_from_nonoverlapping(s_ptr, s_len);
                }
                self.length += s_len;
            }
            _ => panic!("{:?} assumption isn't relevent in this context", assumption),
        }
    }
    
    pub fn push(&mut self, c: char) {
        let mut buf = [0u8; 4];
        let s_char = c.encode_utf8(&mut buf);
        self.push_str(s_char);
    }

    pub fn reserve(&mut self, additional: usize) {
        let curr_capacity = self.capacity();
        let new_capacity = curr_capacity + additional;
        let reallocated = self.force_heap_relocation(new_capacity);
        if !reallocated {
            let layout = unsafe { Layout::from_size_align_unchecked(new_capacity, 4) };
               
            unsafe {
                self.pointer = alloc::realloc(self.pointer, layout, new_capacity);
            }
            self.set_capacity(new_capacity);
        }
    }

    pub fn split<'a>(&'a self, pat: &'a str) -> str::Split<'a, &'a str> {
        self.as_str().split(pat)
    }

    pub fn split_ascii_whitespace(&self) -> str::SplitAsciiWhitespace<'_>{
        self.as_str().split_ascii_whitespace()
    }

    pub fn split_once(&self, delimiter: &str) -> Option<(&str, &str)> {
        self.as_str().split_once(delimiter)
    }

    pub fn rsplit_once(&self, delimiter: &str) -> Option<(&str, &str)> {
        self.as_str().rsplit_once(delimiter)
    }
    
    pub fn split_at(&self, mid: usize) -> (&str, &str) {
        self.as_str().split_at(mid)
    }

    pub fn split_at_checked(&self, mid: usize) -> Option<(&str, &str)> {
        self.as_str().split_at_checked(mid)
    }

    pub fn chars(&self) -> str::Chars<'_> {
        self.as_str().chars()
    }

    pub fn char_indices(&self) -> str::CharIndices<'_> {
        self.as_str().char_indices()
    }

    pub fn contains(&self, pat: &str) -> bool {
        self.as_str().contains(pat)
    }

    pub fn starts_with(&self, pat: &str) -> bool {
        self.as_str().starts_with(pat)
    }

    pub fn ends_with(&self, pat: &str) -> bool {
        self.as_str().ends_with(pat)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        if self.is_inlined() {
            self.inline_ptr()
        } else {
            self.pointer
        }
    }

    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        if self.is_inlined() {
            self.inline_ptr() as *mut u8
        } else {
            self.pointer
        }
    }

    #[inline]
    pub fn as_bytes<'a>(&'a self) -> &'a [u8] {
        let ptr = self.as_ptr();
        let length = self.len();
        let bytes = unsafe { slice::from_raw_parts(ptr, length) };
        bytes
    }

    #[inline]
    pub fn as_bytes_mut<'a>(&'a mut self) -> &'a mut [u8]{
        let ptr = self.as_mut_ptr();
        let length = self.len();
        let bytes = unsafe { slice::from_raw_parts_mut(ptr, length) };
        bytes
    }

    #[inline]
    pub fn as_str<'a>(&'a self) -> &'a str {
        // str::from_utf8(self.as_bytes()).unwrap()

        let md = self.metadata().data;
        unsafe  {
            if md >> 7 == 1 {
                std::str::from_utf8_unchecked(
                    std::slice::from_raw_parts(self.inline_ptr(), (md & 0b000_11111) as usize)
                )
            }
            else {
                std::str::from_utf8_unchecked(
                    std::slice::from_raw_parts(self.pointer, self.length)
                )
            }
        }
    }

    #[inline]
    pub unsafe fn as_str_assume<'a, const PRECOND: SsosPrecondType>(&'a self) -> &'a str {
        let assumption= SsosPrecond::from(PRECOND);
        match assumption {
            SsosPrecond::Heap |
            SsosPrecond::HeapAssumeCapacity |
            SsosPrecond::Static => {
                unsafe {
                    std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.pointer, self.length))
                }
            }
            SsosPrecond::Inline |
            SsosPrecond::InlineAssumeCapacity => {
                unsafe {
                    let md = (self as *const SsoString as *const u8).read();
                    let ptr = self.inline_ptr();
                    let len = (md & 0b000_11111) as usize;
                    std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len))
                }
            }
            _ => panic!("{:?} assumption isn't relevent in this context", assumption),
        }
    }

    pub fn to_string(&self) -> String {
        self.as_str().to_string()
    }

    #[inline]
    fn metadata(&self) -> &SsoStrMetadata {
        let metadata = &self.capacity as *const usize as *const u8;
        unsafe { (metadata.add(Self::MD_OFFSET) as *const SsoStrMetadata).as_ref().unwrap_unchecked() }
    }


    #[inline]
    fn metadata_mut(&mut self) -> &mut SsoStrMetadata {
        let metadata = &mut self.capacity as *mut usize as *mut u8;
        unsafe { (metadata.add(Self::MD_OFFSET) as *mut SsoStrMetadata).as_mut().unwrap_unchecked() }
    }

    #[inline]
    fn set_metadata(&mut self, md: usize) {
        match mem::size_of::<usize>() {
            4 => self.capacity = self.capacity & (Self::BIT_MASK_LOWER_24 as usize),
            8 => self.capacity = self.capacity & (Self::BIT_MASK_LOWER_56 as usize),
            _ => unsafe { hint::unreachable_unchecked() }
        }

        self.capacity = self.capacity | (md << (Self::MD_OFFSET * 8));
    }

    #[inline]
    fn inline_ptr(&self) -> *const u8 {
        self as *const Self as *const u8
    }

    #[inline]
    fn inline_ptr_mut(&mut self) -> *mut u8 {
        self as *mut Self as *mut u8
    }

    #[inline]
    fn set_capacity(&mut self, capacity: usize) {
        match mem::size_of::<usize>() {
            4 => self.capacity = self.capacity & (!Self::BIT_MASK_LOWER_24 as usize),
            8 => self.capacity = self.capacity & (!Self::BIT_MASK_LOWER_56 as usize),
            _ => unsafe { hint::unreachable_unchecked() }
        }
        
        self.capacity = self.capacity | capacity;
    }

    #[inline]
    fn is_heap_allocated(&self) -> bool {
        let md = self.metadata().data;
        md >> 6 == 0
    }

    /// Does nothing if the string is already heap-allocated.
    fn force_heap_relocation(&mut self, capacity: usize) -> bool {
        if self.is_heap_allocated() {
            return false;
        }

        let placeholder = self.clone();
        let layout = Layout::from_size_align(capacity, 4)
            .unwrap();
        let ptr = unsafe { alloc::alloc(layout) };

        self.set_capacity(capacity);
        self.metadata_mut().zero_flags();

        self.length = placeholder.len();
        self.pointer = ptr;

        let src_pointer = placeholder.as_ptr();
        unsafe { 
            ptr.copy_from_nonoverlapping(src_pointer, placeholder.len()) 
        };
        true
    }

    const fn null_string() -> Self {
        SsoString { capacity: 0, length: 0, pointer: 0 as *mut u8 }
    }
}

impl Default for SsoString {
    fn default() -> Self {
        let mut string = Self::null_string();
        string.metadata_mut().set_is_inlined(1);
        string
    }
}

impl fmt::Debug for SsoString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            // Detailed, pretty-printed output for "{:#?}"
            f.debug_struct("SsoString")
                .field("content", &self.as_str()) // self.as_str() will use str's Debug impl (quoted, escaped)
                .field("len", &self.len())
                .field("capacity", &self.capacity())
                .field("is_inlined", &self.is_inlined())
                .field("is_static_flag", &(self.metadata().is_static()))
                .field("metadata_byte", &self.metadata().data) // Raw metadata byte for deeper inspection
                .finish()
        } else {
            // Default output for "{:?}": mimic String's debug output
            // This defers to the Debug implementation for &str,
            // which prints the string quoted and with escaped characters.
            fmt::Debug::fmt(self.as_str(), f)
        }
    }
}

impl Clone for SsoString {
    fn clone(&self) -> Self {
        let mut new_string: SsoString = unsafe { (self as *const SsoString).read() };
        
        if self.is_heap_allocated()  {
            let layout = unsafe { Layout::from_size_align_unchecked(self.capacity(), 4) };
            let ptr = unsafe { alloc::alloc(layout) };
            unsafe { ptr.copy_from_nonoverlapping(self.pointer, self.len()) };
            new_string.pointer = ptr;
        }
        new_string
    }
}

impl Drop for SsoString {
    fn drop(&mut self) {
        if self.is_heap_allocated() {
            let layout = unsafe { Layout::from_size_align_unchecked(self.capacity(), 4) };
            unsafe { alloc::dealloc(self.pointer, layout) };
        }
    }
}

impl Hash for SsoString {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(self.as_bytes());
    }
}

impl AsRef<str> for SsoString {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Into<String> for SsoString {
    fn into(self) -> String {
        self.to_string()
    }
}

impl From<&'static str> for SsoString {
    fn from(value: &'static str) -> Self {
        Self::from_static(value)
    }
}

impl From<String> for SsoString {
    fn from(value: String) -> Self {
        let mut value = mem::ManuallyDrop::new(value);

        Self {
            capacity: value.capacity(),
            length: value.len(),
            pointer: value.as_mut_ptr(),
        }
    }
}

impl PartialEq for SsoString {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for SsoString {}

impl PartialOrd for SsoString {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.as_str().partial_cmp(other.as_str())
    }
}

impl Ord for SsoString {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialEq<String> for SsoString {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<&str> for SsoString {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialOrd<String> for SsoString {
    fn partial_cmp(&self, other: &String) -> Option<std::cmp::Ordering> {
        self.as_str().partial_cmp(other.as_str())
    }
}

impl PartialOrd<&str> for SsoString {
    fn partial_cmp(&self, other: &&str) -> Option<std::cmp::Ordering> {
        self.as_str().partial_cmp(other)
    }
}

impl std::ops::Deref for SsoString {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl SsoStringable for String {
    fn to_sso_string(&self) -> SsoString {
        SsoString::from(self)
    }
}

impl SsoStringable for &str {
    fn to_sso_string(&self) -> SsoString {
        SsoString::from(self)
    }
}
