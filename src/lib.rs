#![feature(integer_atomics, allocator_api, alloc_layout_extra, slice_ptr_get)]

#[macro_use]
extern crate lazy_static;

use std::alloc::{Allocator, Global, Layout, handle_alloc_error};
use std::borrow::{Borrow, Cow};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;

use parking_lot::Mutex;

mod map;

pub use self::map::*;

lazy_static!{
    static ref SYMBOLS: Mutex<HashSet<Symbol>> = {
        let mut set = HashSet::new();
        set.insert(Symbol::alloc("", true));
        Mutex::new(set)
    };
}


struct SymbolHdr {
    ref_count: AtomicUsize,
    ptr: NonNull<u8>,
    len: usize,
}

impl AsRef<str> for SymbolHdr {
    fn as_ref(&self) -> &str {
        unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.ptr.as_ptr(), self.len))
        }
    }
}


#[inline]
fn layout_offset(len: usize) -> (Layout, usize) {
    unsafe {
        Layout::new::<SymbolHdr>().extend(Layout::from_size_align_unchecked(len, 1)).unwrap()
    }
}


pub struct Symbol(NonNull<u8>);

impl Symbol {
    #[inline(never)]
    pub fn get<S: AsRef<str>>(value: S) -> Option<Symbol> {
        let symbols = SYMBOLS.lock();
        symbols.get(value.as_ref()).cloned()
    }

    #[inline(never)]
    pub fn new<S: AsRef<str>>(value: S) -> Symbol {
        let mut symbols = SYMBOLS.lock();
        let value = value.as_ref();
        match symbols.get(value) {
            Some(s) => s.clone(),
            None => {
                let s = Symbol::alloc(value, false);
                let p = s.0;
                symbols.insert(s);
                Symbol(p)
            }
        }
    }

    fn alloc(value: &str, persistent: bool) -> Symbol {
        let (layout, offset) = layout_offset(value.len());
        let p = unsafe {
            let data = Global.allocate(layout).unwrap_or_else(|_| handle_alloc_error(layout));
            let str_ptr = data.as_non_null_ptr().as_ptr().offset(offset as isize);
            let hdr_ptr = std::mem::transmute::<NonNull<u8>, &mut SymbolHdr>(data.as_non_null_ptr());
            *hdr_ptr = SymbolHdr {
                ref_count: AtomicUsize::new(if persistent { 2 } else { 1 }),
                ptr: NonNull::new_unchecked(str_ptr),
                len: value.len(),
            };
            std::ptr::copy_nonoverlapping(value.as_ptr(), str_ptr, value.len());
            data.as_non_null_ptr()
        };
        Symbol(p)
    }

    #[inline(never)]
    fn destroy(&mut self) {
        let mut symbols = SYMBOLS.lock();
        symbols.remove(self);

        let (layout, _) = layout_offset(self.header().len);
        unsafe {
            Global.deallocate(self.0, layout);
        }
    }

    #[inline(always)]
    fn header(&self) -> &SymbolHdr {
        unsafe { std::mem::transmute::<NonNull<u8>, &SymbolHdr>(self.0) }
    }

    #[cfg(test)]
    fn ref_count(&self) -> usize {
        self.header().ref_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for Symbol {
    #[inline(always)]
    fn drop(&mut self) {
        if self.header().ref_count.fetch_sub(1, std::sync::atomic::Ordering::Release) != 1 {
            return;
        }

        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);

        self.destroy();
    }
}

impl Clone for Symbol {
    #[inline(always)]
    fn clone(&self) -> Self {
        self.header().ref_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Symbol(self.0)
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        self.header().as_ref()
    }
}

impl Deref for Symbol {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl Borrow<str> for Symbol {
    fn borrow(&self) -> &str {
        self.as_ref()
    }
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Symbol) -> bool {
        self.0 == other.0
    }
}

impl Eq for Symbol {}

impl PartialOrd for Symbol {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Symbol {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.eq(other) {
            Ordering::Equal
        } else {
            self.as_ref().cmp(&other.as_ref())
        }
    }
}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.as_ref() == other
    }
}

impl<'a> PartialEq<&'a str> for Symbol {
    fn eq(&self, other: &&'a str) -> bool {
        self.as_ref() == *other
    }
}

impl PartialEq<String> for Symbol {
    fn eq(&self, other: &String) -> bool {
        self.as_ref() == other.as_str()
    }
}

impl<'a> PartialEq<Cow<'a, str>> for Symbol {
    fn eq(&self, other: &Cow<'a, str>) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl PartialOrd<str> for Symbol {
    fn partial_cmp(&self, other: &str) -> Option<Ordering> {
        self.as_ref().partial_cmp(other)
    }
}

impl<'a> PartialOrd<&'a str> for Symbol {
    fn partial_cmp(&self, other: &&'a str) -> Option<Ordering> {
        self.as_ref().partial_cmp(*other)
    }
}

impl PartialOrd<String> for Symbol {
    fn partial_cmp(&self, other: &String) -> Option<Ordering> {
        self.as_ref().partial_cmp(other.as_str())
    }
}

impl<'a> PartialOrd<Cow<'a, str>> for Symbol {
    fn partial_cmp(&self, other: &Cow<'a, str>) -> Option<Ordering> {
        self.as_ref().partial_cmp(other.as_ref())
    }
}

impl std::fmt::Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_ref(), f)
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt(self.as_ref(), f)
    }
}

impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state)
    }
}

impl Default for Symbol {
    fn default() -> Self {
        Symbol::new("")
    }
}

impl<'a> From<&'a Symbol> for Symbol {
    fn from(s: &'a Symbol) -> Self {
        s.clone()
    }
}

impl From<String> for Symbol {
    fn from(s: String) -> Self {
        Symbol::new(s)
    }
}

impl<'a> From<&'a String> for Symbol {
    fn from(s: &'a String) -> Self {
        Symbol::new(s)
    }
}

impl<'a> From<&'a str> for Symbol {
    fn from(s: &'a str) -> Self {
        Symbol::new(s)
    }
}

impl<'a> From<Cow<'a, str>> for Symbol {
    fn from(s: Cow<'a, str>) -> Self {
        Symbol::new(s)
    }
}

impl<'a, 'b> From<&'b Cow<'a, str>> for Symbol {
    fn from(s: &'b Cow<'a, str>) -> Self {
        Symbol::new(s)
    }
}

impl heapsize::HeapSizeOf for Symbol {
    fn heap_size_of_children(&self) -> usize {
        layout_offset(self.header().len).0.size()
    }
}

impl serde::Serialize for Symbol {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: serde::Serializer {
        self.as_ref().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Symbol {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: serde::Deserializer<'de> {
        Ok(Symbol::from(String::deserialize(deserializer)?))
    }
}

unsafe impl Send for Symbol {}

unsafe impl Sync for Symbol {}


#[cfg(test)]
mod tests {
    use parking_lot::{Mutex, MutexGuard};
    use super::*;

    // Some tests must be run consecutively (not in parallel), so we need to test_lock() before each test
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    pub(crate) fn test_lock<'a>() -> MutexGuard<'a, ()> {
        let lock = TEST_LOCK.lock();
        debug_assert_eq!(SYMBOLS.lock().len(), 1);
        lock
    }

    #[test]
    fn ptr_equality() {
        let _lock = test_lock();

        let s1 = Symbol::from("aaa");
        let s2 = Symbol::from("aaa");
        let s3 = s1.clone();
        let s4 = Symbol::from("aaaa");

        assert_eq!(s1.0, s2.0);
        assert_eq!(s1.0, s3.0);
        assert_ne!(s1.0, s4.0);
    }

    #[test]
    fn symbols_are_dropped() {
        let _lock = test_lock();

        {
            let _s1 = Symbol::from("aaa");
            let s2 = Symbol::from("aaa");
            let s3 = Symbol::from("aaaa");
            assert_eq!(s2.ref_count(), 2);
            assert_eq!(s3.ref_count(), 1);
            assert_eq!(SYMBOLS.lock().len(), 3);
        }

        assert_eq!(SYMBOLS.lock().len(), 1);
    }

    #[test]
    fn symbol_keys_in_maps() {
        let _lock = test_lock();

        use std::collections::HashMap;

        let mut map: HashMap<Symbol, usize> = HashMap::new();
        map.insert("one".into(), 1);
        map.insert("two".into(), 2);
        map.insert("three".into(), 3);

        let three = Symbol::new("three");

        assert_eq!(map.get("one"), Some(&1));
        assert_eq!(map.get("two"), Some(&2));
        assert!(map.contains_key(&three))
    }

    #[test]
    fn serialize() {
        let _lock = test_lock();

        let s = Symbol::from("example");
        let json = serde_json::to_string_pretty(&s).unwrap();
        assert_eq!("\"example\"", json);
    }

    #[test]
    fn deserialize() {
        let _lock = test_lock();

        let json = "\"example\"";
        let s: Symbol = serde_json::from_str(json).unwrap();
        assert_eq!(s.as_ref(), "example");
    }

    #[test]
    fn symbol_is_sync() {
        let _lock = test_lock();

        fn test<T: Sync>(_: T) {}

        test(Symbol::from("example"));
    }

    #[test]
    fn symbol_is_send() {
        let _lock = test_lock();

        fn test<T: Send>(_: T) {}

        test(Symbol::from("example"));
    }

    #[test]
    fn symbol_hash_eq_str_hash() {
        use std::collections::hash_map::DefaultHasher;

        let _lock = test_lock();

        let s1 = "example string";
        let h1 = {
            let mut hasher = DefaultHasher::new();
            s1.hash(&mut hasher);
            hasher.finish()
        };

        let s2 = Symbol::new(s1);
        let h2 = {
            let mut hasher = DefaultHasher::new();
            s2.hash(&mut hasher);
            hasher.finish()
        };

        assert_eq!(h1, h2);
    }

    #[test]
    fn symbol_sizeof_is_equal_to_pointer() {
        // can be run in parallel
        assert_eq!(std::mem::size_of::<Symbol>(), std::mem::size_of::<*const ()>());
    }

    #[test]
    fn optional_symbol_sizeof_is_equal_to_pointer() {
        // can be run in parallel
        assert_eq!(std::mem::size_of::<Option<Symbol>>(), std::mem::size_of::<*const ()>());
    }
}



