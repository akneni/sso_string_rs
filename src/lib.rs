use std::{alloc::{self, alloc}, fmt, hash::Hash, hint, mem, slice, str};

/// Bits 0-4: Length of the string (if inlined)
/// Bit 5: Empty (reserved for future use??)
/// Bit 6: flag IS_STATIC. Any operations requiring mutable state need to copy the underlying data if this bit is set to 1 (this flag is irrelevent for inlined strings). Enables us to do CoW optimizations. 
/// Bit 7: IS_INLINED. 
struct SsoStrMetadata {
    data: u8,
}

pub struct SsoString {
    capacity: usize,
    length: usize,
    pointer: *mut u8,
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
        (self.data & 0b100_00000) >> 7 == 1
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
    fn zero_all(&mut self) {
        self.data = 0;
    }

    #[inline]
    #[allow(unused)]
    fn zero_flags(&mut self) {
        self.data = self.data &0b000_11111;
    }
}


impl SsoString {
    const BIT_MASK_LOWER_U32_24: u32 = 0xFFFFFF;
    const BIT_MASK_LOWER_U64_56: u64 = 0xFFFFFFFFFFFFFF;

    const INLINE_CAPACITY: usize = 23;

    pub fn from(s: impl AsRef<str>) -> Self {
        let s = s.as_ref();
        if s.len() > Self::INLINE_CAPACITY {
            let layout = alloc::Layout::from_size_align(s.len(), 4)
                .unwrap();
    
            let string = SsoString { 
                capacity: s.len(), 
                length: s.len(), 
                pointer: unsafe { alloc(layout) } 
            };

            unsafe {
                string.pointer.copy_from_nonoverlapping(s.as_ptr(), s.len());
            }

            return string;
        }

        let mut string = Self::null_string();
        let metadata = string.metadata_mut();
        metadata.zero_all();
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
            4 => {
                self.capacity & Self::BIT_MASK_LOWER_U32_24 as usize
            }
            8 => {
                self.capacity & Self::BIT_MASK_LOWER_U64_56 as usize
            }
            _ => unsafe { hint::unreachable_unchecked() },
        }

    }

    #[inline]
    pub fn is_inlined(&self) -> bool {
        self.metadata().is_inlined()
    }

    pub fn push_str(&mut self, s: &str) {
        let s_len = s.len();
        let self_len = self.len();
        let new_length = self_len + s_len;

        let md = self.metadata();

        let is_inlined = md.is_inlined();
        let is_static = md.is_static();
        let fits_inline = new_length <= Self::INLINE_CAPACITY;

        if is_inlined && fits_inline {
            let ptr = unsafe { self.inline_ptr_mut().add(self_len) };
            unsafe {
                ptr.copy_from_nonoverlapping(s.as_ptr(), s_len);
            }
            self.metadata_mut().set_inline_len(new_length as u8);
        }
        else if is_static && fits_inline {
            let static_ptr = self.pointer as *const u8;

            let inline_ptr = self.inline_ptr_mut();
            unsafe {
                inline_ptr.copy_from_nonoverlapping(static_ptr, self_len);
                inline_ptr.add(self_len).copy_from_nonoverlapping(s.as_ptr(), s_len);
            }
            let md = self.metadata_mut();
            md.data = 0b100_00000 | new_length as u8;
        }
        else if is_inlined || is_static {
            self.force_heap_relocation(new_length * 3 / 2);
            let ptr = unsafe { self.pointer.add(self_len) };

            unsafe {
                ptr.copy_from_nonoverlapping(s.as_ptr(), s_len);
            }
            self.length = new_length;
        }
        else if new_length > self.capacity() {
            let new_capacity = new_length * 3 / 2;
            self.reserve(new_capacity - self.capacity());
            let ptr = unsafe { self.pointer.add(self_len) };
            unsafe {
                ptr.copy_from_nonoverlapping(s.as_ptr(), s_len);
            }
            self.length = new_length;
        }
        else {
            let ptr = unsafe { self.pointer.add(self_len) };
            unsafe {
                ptr.copy_from_nonoverlapping(s.as_ptr(), s_len);
            }
            self.length = new_length;
        }

    }

    pub fn reserve(&mut self, additional: usize) {
        let new_capacity = self.capacity() + additional;
        let reallocated = self.force_heap_relocation(new_capacity);
        if !reallocated {
            let layout = alloc::Layout::from_size_align(new_capacity, 4)
                .unwrap();
            let ptr = unsafe { alloc::alloc(layout) };
            unsafe {
                ptr.copy_from_nonoverlapping(self.pointer, self.len());
                alloc::dealloc(
                    self.pointer, 
                    alloc::Layout::from_size_align(self.capacity(), 4).unwrap()
                );
            }
            self.pointer = ptr;
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
        let md = self.metadata();
        unsafe  {
            if md.is_inlined() {
                std::str::from_utf8_unchecked(
                    std::slice::from_raw_parts(self.inline_ptr(), md.inline_len() as usize)
                )
            }
            else {
                std::str::from_utf8_unchecked(
                    std::slice::from_raw_parts(self.pointer, self.length)
                )
            }
        }
    }

    pub fn to_string(&self) -> String {
        self.as_str().to_string()
    }

    #[inline]
    fn metadata(&self) -> &SsoStrMetadata {
        let metadata = self as *const SsoString as *const SsoStrMetadata;
        unsafe { metadata.as_ref().unwrap_unchecked() }
    }


    #[inline]
    fn metadata_mut(&mut self) -> &mut SsoStrMetadata {
        let metadata = self as *mut SsoString as *mut SsoStrMetadata;
        unsafe { metadata.as_mut().unwrap_unchecked() }
    }

    #[inline]
    fn inline_ptr(&self) -> *const u8 {
        let ptr = self as *const SsoString as *const u8;
        unsafe { ptr.add(1) }
    }

    #[inline]
    fn inline_ptr_mut(&mut self) -> *mut u8 {
        let ptr = self as *mut SsoString as *mut u8;
        unsafe { ptr.add(1) }
    }

    #[inline]
    fn set_capacity(&mut self, capacity: usize) {
        self.capacity = self.capacity & (!Self::BIT_MASK_LOWER_U64_56 as usize);
        self.capacity = self.capacity | capacity;
    }

    #[inline]
    fn is_heap_allocated(&self) -> bool {
        !self.is_inlined() && !self.metadata().is_static()
    }

    /// Does nothing if the string is already heap-allocated.
    fn force_heap_relocation(&mut self, capacity: usize) -> bool {
        if self.is_heap_allocated() {
            return false;
        }

        let placeholder = self.clone();
        let layout = alloc::Layout::from_size_align(capacity, 4)
            .unwrap();
        let ptr = unsafe { alloc::alloc(layout) };

        self.set_capacity(capacity);

        let md = self.metadata_mut();
        md.zero_flags();

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
        let mut new_string: SsoString = SsoString::null_string();

        unsafe {
            (&mut new_string as *mut SsoString).copy_from_nonoverlapping(
                self as *const SsoString,
                1
            );
        }

        if self.is_heap_allocated()  {
            let layout = alloc::Layout::from_size_align(self.capacity(), 4)
                .unwrap();
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
            let layout = alloc::Layout::from_size_align(self.capacity(), 4)
                .unwrap();
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


#[cfg(test)]
mod private_tests {
    use super::*;

    #[test]
    fn test_from_static() {
        let static_str_short_literal = "static short";
        let s_static_inline = SsoString::from_static(static_str_short_literal);
        assert!(s_static_inline.metadata().is_static());
        assert_eq!(s_static_inline.len(), static_str_short_literal.len());
        assert_eq!(s_static_inline.as_str(), static_str_short_literal);

        let static_str_long_literal = "this is a longer static string that will exceed inline capacity for sure";
        let s_static_heap = SsoString::from_static(static_str_long_literal);
        assert!(!s_static_heap.is_inlined());
        assert_eq!(s_static_heap.len(), static_str_long_literal.len());
        assert_eq!(s_static_heap.as_str(), static_str_long_literal);
        assert_eq!(s_static_heap.metadata().is_static(), true);
    }

    #[test]
    fn test_push_str_static_heap_becomes_mutable_heap() {
        let main_static_literal = "0123456789_0123456789_0123456789_static";
        let to_push_literal = "_plus_this";
        let mut s = SsoString::from_static(main_static_literal);
        assert!(!s.is_inlined());
        assert_eq!(s.metadata().is_static(), true, "Initially static heap");

        s.push_str(to_push_literal);
        assert!(!s.is_inlined(), "Should remain on heap");
        assert_eq!(s.metadata().is_static(), false, "Should become non-static after push_str");
        let expected_str_obj = String::from(main_static_literal) + to_push_literal;
        assert_eq!(s.as_str(), expected_str_obj.as_str());
        assert_eq!(s.len(), expected_str_obj.len());
    }

    #[test]
    fn test_clone() {
        let inline_literal = "clone_me_inline";
        let s1_inline = SsoString::from(inline_literal);
        let s2_inline = s1_inline.clone();
        assert!(s2_inline.is_inlined());
        assert_eq!(s1_inline.as_str(), s2_inline.as_str());
        assert_eq!(s1_inline.len(), s2_inline.len());

        let heap_literal = "clone_me_heap_because_i_am_a_long_string";
        let s1_heap = SsoString::from(heap_literal);
        let s2_heap = s1_heap.clone();
        assert!(!s1_heap.is_inlined());
        assert!(!s2_heap.is_inlined());
        assert!(!s1_heap.metadata().is_static());
        assert!(!s2_heap.metadata().is_static());
        assert_eq!(s1_heap.as_str(), s2_heap.as_str());
        assert_ne!(s1_heap.pointer, s2_heap.pointer, "Heap clone should have different pointer");

        let static_data_literal = "clone_me_static_heap_long_string";
        let s1_static_heap = SsoString::from_static(static_data_literal);
        let s2_static_heap = s1_static_heap.clone();
        assert!(!s2_static_heap.is_inlined());
        assert_eq!(s1_static_heap.as_str(), s2_static_heap.as_str());
        assert_eq!(s1_static_heap.metadata().is_static(), true);
        assert_eq!(s2_static_heap.metadata().is_static(), true, "Clone of static string should also be marked static initially");
        if s1_static_heap.pointer != 0 as *mut u8 {
             assert_eq!(s1_static_heap.pointer, s2_static_heap.pointer, "Clone of static heap string should share pointer until CoW");
        }
    }
}