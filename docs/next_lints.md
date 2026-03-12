# Good next lints to add

1. `stack_friendly_fixed_buffer`
   - detect `Vec::with_capacity(CONST)` plus only indexed writes up to a small constant
   - suggest `[T; N]` or `ArrayVec`

2. `cache_sensitive_indirection`
   - detect `Vec<Box<T>>`, `Vec<Rc<T>>`, `HashMap<K, Box<V>>` in modules marked `#[cache_sensitive]`

3. `large_hot_struct`
   - warn when a struct marked `#[cache_sensitive]` exceeds one 64-byte cache line

4. `field_order_padding`
   - warn when field ordering creates avoidable padding

5. `alloc_in_hot_loop`
   - warn on `Vec::push`, `String::push_str`, `Box::new`, `format!` inside loops/functions marked `#[hot_path]`
