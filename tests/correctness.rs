use sso_string::SsoString;

#[cfg(test)]
mod correctness_tests {
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