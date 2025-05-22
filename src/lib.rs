use std::{alloc::{self, alloc}, fmt::{self, Debug}, hash::Hash, hint, mem, slice, str};

/// Bits 0-4: Length of the string (if inlined)
/// Bit 5: IS_ASCII. Not yet used, but may allow us to make optimizations if we can assert that string only contains ascii characters. 
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

trait SsoStringable {
    fn to_sso_string(&self) -> SsoString;
}

impl SsoStrMetadata {
    #[inline]
    fn inline_len(&self) -> u8 {
        self.data & 0b000_11111
    }

    #[inline]
    fn is_ascii(&self) -> bool {
        (self.data & 0b001_00000) >> 5 == 1
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
    fn set_is_ascii(&mut self, flag: u8) {
        self.data = self.data & 0b110_11111;
        self.data = self.data | (flag << 5);
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
    fn zero_flags(&mut self) {
        self.data = self.data &0b000_11111;
    }
}


impl SsoString {
    const BIT_MASK_LOWER_24: u32 = 0xFFFFFF;
    
    const BIT_MASK_LOWER_56: u64 = 0xFFFFFFFFFFFFFF;
    const BIT_MASK_UPPER_8: u64 = 0xFF << 56;

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

    pub fn from_static(s: &'static str) -> Self {
        if s.len() > Self::INLINE_CAPACITY {    
            let mut string = SsoString { 
                capacity: s.len(), 
                length: s.len(), 
                pointer: s.as_ptr() as *mut u8,
            };

            let md = string.metadata_mut();
            md.set_is_static(1);

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
                self.capacity & Self::BIT_MASK_LOWER_24 as usize
            }
            8 => {
                self.capacity & Self::BIT_MASK_LOWER_56 as usize
            }
            _ => unsafe { hint::unreachable_unchecked() },
        }

    }

    #[inline]
    pub fn is_inlined(&self) -> bool {
        self.metadata().is_inlined()
    }

    pub fn push_str(&mut self, s: &str) {
        let new_length = self.len() + s.len();

        if self.is_inlined() && new_length <= Self::INLINE_CAPACITY {
            let ptr = unsafe { self.inline_ptr_mut().add(self.len()) };
            unsafe {
                ptr.copy_from_nonoverlapping(s.as_ptr(), s.len());
            }
            self.metadata_mut().set_inline_len(new_length as u8);
        }
        else if self.is_inlined() || self.metadata().is_static() {
            self.force_heap_relocation(new_length * 3 / 2);
            let ptr = unsafe { self.pointer.add(self.len()) };

            unsafe {
                ptr.copy_from_nonoverlapping(s.as_ptr(), s.len());
            }
            self.length = new_length;
        }
        else if new_length > self.capacity() {
            let new_capacity = new_length * 3 / 2;
            self.reserve(new_capacity - self.capacity());
            let ptr = unsafe { self.pointer.add(self.len()) };
            unsafe {
                ptr.copy_from_nonoverlapping(s.as_ptr(), s.len());
            }
            self.length = new_length;
        }
        else {
            let ptr = unsafe { self.pointer.add(self.len()) };
            unsafe {
                ptr.copy_from_nonoverlapping(s.as_ptr(), s.len());
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

        // let arr = [self.pointer, self.inline_ptr()];
        // let idx = (self.metadata().data & 0b100_00000) >> 7;
        // arr[idx as usize]
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
        let bytes = self.as_bytes();
        unsafe { std::str::from_utf8_unchecked(bytes) }
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
        self.capacity = self.capacity & (!Self::BIT_MASK_LOWER_56 as usize);
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
        self.metadata_mut().set_is_inlined(0);
        self.metadata_mut().set_is_static(0);

        self.length = placeholder.len();
        self.pointer = ptr;

        let src_pointer = if placeholder.is_inlined() {
            placeholder.inline_ptr()
        } else {
            placeholder.pointer
        };

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
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_init() {
        let static_str_0 = "small string";
        let static_str_1 = "0123456789_0123456789_0123456789";
        let static_str_2 = "12345678901234567890123";
        let static_str_4 = "123456789012345678901234";
        let empty_str_literal = "";

        let s_inline = SsoString::from(static_str_0);
        assert!(s_inline.is_inlined());
        assert_eq!(s_inline.len(), static_str_0.len());
        assert_eq!(s_inline.as_str(), static_str_0);

        let s_heap = SsoString::from(static_str_1);
        assert!(!s_heap.is_inlined());
        assert_eq!(s_heap.len(), static_str_1.len());
        assert_eq!(s_heap.as_str(), static_str_1);

        let s_empty = SsoString::from(empty_str_literal);
        assert!(s_empty.is_inlined());
        assert_eq!(s_empty.len(), empty_str_literal.len());
        assert_eq!(s_empty.as_str(), empty_str_literal);

        let s_max_inline = SsoString::from(static_str_2);
        assert!(s_max_inline.is_inlined());
        assert_eq!(s_max_inline.len(), static_str_2.len());
        assert_eq!(s_max_inline.capacity(), static_str_2.len());
        assert_eq!(s_max_inline.as_str(), static_str_2);

        let s_min_heap = SsoString::from(static_str_4);
        assert!(!s_min_heap.is_inlined());
        assert_eq!(s_min_heap.len(), static_str_4.len());
        assert_eq!(s_min_heap.as_str(), static_str_4);
    }

    #[test]
    fn test_from_static() {
        let static_str_short_literal = "static short";
        let s_static_inline = SsoString::from_static(static_str_short_literal);
        assert!(s_static_inline.is_inlined());
        assert_eq!(s_static_inline.len(), static_str_short_literal.len());
        assert_eq!(s_static_inline.as_str(), static_str_short_literal);
        assert_eq!(s_static_inline.metadata().is_static(), false);

        let static_str_long_literal = "this is a longer static string that will exceed inline capacity for sure";
        let s_static_heap = SsoString::from_static(static_str_long_literal);
        assert!(!s_static_heap.is_inlined());
        assert_eq!(s_static_heap.len(), static_str_long_literal.len());
        assert_eq!(s_static_heap.as_str(), static_str_long_literal);
        assert_eq!(s_static_heap.metadata().is_static(), true);
    }

    #[test]
    fn test_push_str_inline_to_inline() {
        let initial_str = "hello";
        let to_push_str = " world";
        let expected_str = "hello world";
        let mut s = SsoString::from(initial_str);
        assert!(s.is_inlined());
        s.push_str(to_push_str);
        assert!(s.is_inlined());
        assert_eq!(s.as_str(), expected_str);
        assert_eq!(s.len(), expected_str.len());
    }

    #[test]
    fn test_push_str_inline_to_heap() {
        let initial_str = "12345678901234567890";
        let to_push_str = "abcde";
        let expected_str = "12345678901234567890abcde";
        let mut s = SsoString::from(initial_str);
        assert!(s.is_inlined());
        s.push_str(to_push_str);
        assert!(!s.is_inlined(), "String should have moved to heap");
        assert_eq!(s.as_str(), expected_str);
        assert_eq!(s.len(), expected_str.len());
    }

    #[test]
    fn test_push_str_heap_to_heap() {
        let initial_str = "This is a long string, initially on the heap.";
        let to_push_str = " And now it's even longer.";
        let expected_str_val = initial_str.to_string() + to_push_str;
        let mut s = SsoString::from(initial_str);
        assert!(!s.is_inlined());
        s.push_str(to_push_str);
        assert!(!s.is_inlined());
        assert_eq!(s.as_str(), expected_str_val.as_str());
        assert_eq!(s.len(), expected_str_val.len());
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
    fn test_push_empty_str() {
        let initial_inline_str = "test";
        let empty_str_literal = "";
        let mut s_inline = SsoString::from(initial_inline_str);
        s_inline.push_str(empty_str_literal);
        assert_eq!(s_inline.as_str(), initial_inline_str);
        assert_eq!(s_inline.len(), initial_inline_str.len());

        let initial_heap_str = "this is a much longer string on the heap for testing";
        let mut s_heap = SsoString::from(initial_heap_str);
        let original_len = s_heap.len();
        s_heap.push_str(empty_str_literal);
        assert_eq!(s_heap.as_str(), initial_heap_str);
        assert_eq!(s_heap.len(), original_len);
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
    
    #[test]
    fn test_clone_and_mutate() {
        let original_literal = "original";
        let changed_literal_part = " changed";
        let expected_changed_literal = "original changed";
        let s1 = SsoString::from(original_literal);
        let mut s2 = s1.clone();

        s2.push_str(changed_literal_part);
        assert_eq!(s1.as_str(), original_literal);
        assert_eq!(s2.as_str(), expected_changed_literal);

        let long_original_literal = "this is a long string that will be on the heap for sure";
        let long_changed_part = " and it has been modified";
        let s3 = SsoString::from(long_original_literal);
        let mut s4 = s3.clone();
        s4.push_str(long_changed_part);

        let expected_long_changed_val = long_original_literal.to_string() + long_changed_part;
        assert_eq!(s3.as_str(), long_original_literal);
        assert_eq!(s4.as_str(), expected_long_changed_val.as_str());
    }

    #[test]
    fn test_eq_hash_ord() {
        let hello_literal = "hello";
        let world_literal = "world";
        let long_str_1_literal = "long_string_example_for_testing_equality_and_hash";
        let long_str_2_literal = "another_long_string_for_testing_different_content";
        
        let inline_val_literal = "inline_val";
        let heap_val_literal = "heap_val";

        let s1_inline = SsoString::from(hello_literal);
        let s2_inline = SsoString::from(hello_literal);
        let s3_inline = SsoString::from(world_literal);

        let s1_heap = SsoString::from(long_str_1_literal);
        let s2_heap = SsoString::from(long_str_1_literal);
        let s3_heap = SsoString::from(long_str_2_literal);

        assert_eq!(s1_inline, s2_inline);
        assert_ne!(s1_inline, s3_inline);
        assert_eq!(s1_heap, s2_heap);
        assert_ne!(s1_heap, s3_heap);
        assert_ne!(s1_inline, s1_heap);

        let mut map = HashMap::new();
        map.insert(s1_inline.clone(), inline_val_literal);
        map.insert(s1_heap.clone(), heap_val_literal);

        assert_eq!(map.get(&s2_inline), Some(&inline_val_literal));
        assert_eq!(map.get(&s2_heap), Some(&heap_val_literal));
        assert_eq!(map.get(&s3_inline), None);

        assert!(s1_inline < s3_inline);
        assert!(s3_heap < s1_heap);
        assert_eq!(s1_inline.cmp(&s2_inline), std::cmp::Ordering::Equal);
        assert_eq!(s3_heap.partial_cmp(&s1_heap), Some(std::cmp::Ordering::Less));
    }

    #[test]
    fn test_cmp_with_std_types() {
        let hello_literal = "hello";
        let world_literal = "world";
        let zebra_literal = "zebra";

        let sso_hello = SsoString::from(hello_literal);
        let std_hello_string = String::from(hello_literal);
        let std_hello_str = hello_literal;

        let sso_world = SsoString::from(world_literal);
        let std_zebra_string = String::from(zebra_literal);

        assert_eq!(sso_hello, std_hello_string);
        assert_eq!(sso_hello, std_hello_str);
        assert!(sso_hello < sso_world);
        assert!(sso_hello < std_zebra_string);
        assert!(sso_world.partial_cmp(&std_hello_str) == Some(std::cmp::Ordering::Greater));
    }
    
    #[test]
    fn test_reserve_inline() {
        let initial_literal = "small";
        let to_push_literal = "1234567890";
        let reserve_amount = 10;
        let expected_final_literal = "small1234567890";

        let mut s = SsoString::from(initial_literal);
        assert!(s.is_inlined());
        s.reserve(reserve_amount);
        assert!(!s.is_inlined(), "Reserve on inline string should move to heap");
        assert_eq!(s.as_str(), initial_literal);
        assert!(s.capacity() >= initial_literal.len() + reserve_amount);
        s.push_str(to_push_literal);
        assert_eq!(s.as_str(), expected_final_literal);
    }

    #[test]
    fn test_reserve_heap() {
        let initial_literal = "this is a longer string on the heap";
        let to_push_literal = " plus more text";
        let reserve_amount = 20;
        let expected_final_literal = initial_literal.to_string() + to_push_literal;

        let mut s = SsoString::from(initial_literal);
        assert!(!s.is_inlined());
        let initial_cap = s.capacity();
        let initial_len = s.len();

        s.reserve(reserve_amount);
        assert!(!s.is_inlined());
        assert_eq!(s.len(), initial_len);
        assert!(s.capacity() >= initial_cap + reserve_amount);
        assert_eq!(s.as_str(), initial_literal);

        s.push_str(to_push_literal);
        assert_eq!(s.as_str(), expected_final_literal.as_str());
    }

    #[test]
    fn test_to_string_conversion() {
        let literal_1 = "convert me";
        let sso = SsoString::from(literal_1);
        let std_string = sso.to_string();
        assert_eq!(std_string, literal_1);

        let literal_2 = "a very long string to test conversion from heap";
        let sso_long = SsoString::from(literal_2);
        let std_string_long = sso_long.to_string();
        assert_eq!(std_string_long, literal_2);
    }

    #[test]
    fn test_multiple_pushes_crossing_inline_boundary() {
        let part1 = "abc";
        let part2 = "defghij";
        let part3 = "klmnopqrs";
        let part4 = "tuvwxyz";

        let expected1 = "abcdefghij";
        let expected2 = "abcdefghijklmnopqrs";
        let expected3 = "abcdefghijklmnopqrstuvwxyz";
        
        let mut s = SsoString::from(part1);
        s.push_str(part2);
        assert_eq!(s.as_str(), expected1);
        assert!(s.is_inlined());
        s.push_str(part3);
        assert_eq!(s.as_str(), expected2);
        assert!(s.is_inlined());
        s.push_str(part4);
        assert_eq!(s.as_str(), expected3);
        assert!(!s.is_inlined());
    }

    #[test]
    fn test_push_str_to_exactly_max_inline() {
        let initial_part = "12345678901234567890";
        let second_part = "123";
        let third_part = "4";

        let expected_at_max_inline = "12345678901234567890123";
        let expected_after_overflow = "123456789012345678901234";
        
        let mut s = SsoString::from(initial_part);
        s.push_str(second_part);
        assert!(s.is_inlined());
        assert_eq!(s.len(), expected_at_max_inline.len());
        assert_eq!(s.capacity(), expected_at_max_inline.len());
        assert_eq!(s.as_str(), expected_at_max_inline);
        
        s.push_str(third_part);
        assert!(!s.is_inlined());
        assert_eq!(s.len(), expected_after_overflow.len());
        assert_eq!(s.as_str(), expected_after_overflow);
    }

    #[test]
    fn test_original_push_str_failure_case_logic() {
        let main_str_literal = "0123456789_0123456789_0123456789";
        let other_str_literal = "thing";
        let max_inline_len_defining_str = "12345678901234567890123"; // Used for its length (23)

        let mut s_from = SsoString::from(main_str_literal);
        if main_str_literal.len() > max_inline_len_defining_str.len() && s_from.as_str() != main_str_literal {
            // This block's internal logic remains as is
        }

        s_from.push_str(other_str_literal);
        let mut s2_std = String::from(main_str_literal);
        s2_std.push_str(other_str_literal);
        
        assert_eq!(s_from.len(), s2_std.len());
        assert_eq!(s_from.as_str(), s2_std.as_str());

        let mut s_static = SsoString::from_static(main_str_literal);
        s_static.push_str(other_str_literal);
        assert_eq!(s_static.len(), s2_std.len());
        assert_eq!(s_static.as_str(), s2_std.as_str());
    }
}