// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use core::cmp::Ordering;
use core::fmt::Debug;
use core::hash::{Hash, Hasher};
use core::iter::{FromIterator, Peekable, FusedIterator};
use core::marker::PhantomData;
use core::ops::Index;
use core::{fmt, intrinsics, mem, ptr};

use borrow::Borrow;
use Bound::{Excluded, Included, Unbounded};
use range::RangeArgument;

use super::node::{self, Handle, NodeRef, marker};
use super::search;

use super::node::InsertResult::*;
use super::node::ForceResult::*;
use super::search::SearchResult::*;
use self::UnderflowResult::*;
use self::Entry::*;

/// A map based on a B-Tree.
///
/// B-Trees represent a fundamental compromise between cache-efficiency and actually minimizing
/// the amount of work performed in a search. In theory, a binary search tree (BST) is the optimal
/// choice for a sorted map, as a perfectly balanced BST performs the theoretical minimum amount of
/// comparisons necessary to find an element (log<sub>2</sub>n). However, in practice the way this
/// is done is *very* inefficient for modern computer architectures. In particular, every element
/// is stored in its own individually heap-allocated node. This means that every single insertion
/// triggers a heap-allocation, and every single comparison should be a cache-miss. Since these
/// are both notably expensive things to do in practice, we are forced to at very least reconsider
/// the BST strategy.
///
/// A B-Tree instead makes each node contain B-1 to 2B-1 elements in a contiguous array. By doing
/// this, we reduce the number of allocations by a factor of B, and improve cache efficiency in
/// searches. However, this does mean that searches will have to do *more* comparisons on average.
/// The precise number of comparisons depends on the node search strategy used. For optimal cache
/// efficiency, one could search the nodes linearly. For optimal comparisons, one could search
/// the node using binary search. As a compromise, one could also perform a linear search
/// that initially only checks every i<sup>th</sup> element for some choice of i.
///
/// Currently, our implementation simply performs naive linear search. This provides excellent
/// performance on *small* nodes of elements which are cheap to compare. However in the future we
/// would like to further explore choosing the optimal search strategy based on the choice of B,
/// and possibly other factors. Using linear search, searching for a random element is expected
/// to take O(B log<sub>B</sub>n) comparisons, which is generally worse than a BST. In practice,
/// however, performance is excellent.
///
/// It is a logic error for a key to be modified in such a way that the key's ordering relative to
/// any other key, as determined by the [`Ord`] trait, changes while it is in the map. This is
/// normally only possible through [`Cell`], [`RefCell`], global state, I/O, or unsafe code.
///
/// [`Ord`]: ../../std/cmp/trait.Ord.html
/// [`Cell`]: ../../std/cell/struct.Cell.html
/// [`RefCell`]: ../../std/cell/struct.RefCell.html
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
///
/// // type inference lets us omit an explicit type signature (which
/// // would be `BTreeMap<&str, &str>` in this example).
/// let mut movie_reviews = BTreeMap::new();
///
/// // review some movies.
/// movie_reviews.insert("Office Space",       "Deals with real issues in the workplace.");
/// movie_reviews.insert("Pulp Fiction",       "Masterpiece.");
/// movie_reviews.insert("The Godfather",      "Very enjoyable.");
/// movie_reviews.insert("The Blues Brothers", "Eye lyked it alot.");
///
/// // check for a specific one.
/// if !movie_reviews.contains_key("Les Misérables") {
///     println!("We've got {} reviews, but Les Misérables ain't one.",
///              movie_reviews.len());
/// }
///
/// // oops, this review has a lot of spelling mistakes, let's delete it.
/// movie_reviews.remove("The Blues Brothers");
///
/// // look up the values associated with some keys.
/// let to_find = ["Up!", "Office Space"];
/// for book in &to_find {
///     match movie_reviews.get(book) {
///        Some(review) => println!("{}: {}", book, review),
///        None => println!("{} is unreviewed.", book)
///     }
/// }
///
/// // iterate over everything.
/// for (movie, review) in &movie_reviews {
///     println!("{}: \"{}\"", movie, review);
/// }
/// ```
///
/// `BTreeMap` also implements an [`Entry API`](#method.entry), which allows
/// for more complex methods of getting, setting, updating and removing keys and
/// their values:
///
/// ```
/// use std::collections::BTreeMap;
///
/// // type inference lets us omit an explicit type signature (which
/// // would be `BTreeMap<&str, u8>` in this example).
/// let mut player_stats = BTreeMap::new();
///
/// fn random_stat_buff() -> u8 {
///     // could actually return some random value here - let's just return
///     // some fixed value for now
///     42
/// }
///
/// // insert a key only if it doesn't already exist
/// player_stats.entry("health").or_insert(100);
///
/// // insert a key using a function that provides a new value only if it
/// // doesn't already exist
/// player_stats.entry("defence").or_insert_with(random_stat_buff);
///
/// // update a key, guarding against the key possibly not being set
/// let stat = player_stats.entry("attack").or_insert(100);
/// *stat += random_stat_buff();
/// ```
#[stable(feature = "rust1", since = "1.0.0")]
pub struct BTreeMap<K, V> {
    root: node::Root<K, V>,
    length: usize,
}

#[stable(feature = "btree_drop", since = "1.7.0")]
unsafe impl<#[may_dangle] K, #[may_dangle] V> Drop for BTreeMap<K, V> {
    fn drop(&mut self) {
        unsafe {
            drop(ptr::read(self).into_iter());
        }
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K: Clone, V: Clone> Clone for BTreeMap<K, V> {
    fn clone(&self) -> BTreeMap<K, V> {
        fn clone_subtree<K: Clone, V: Clone>(node: node::NodeRef<marker::Immut,
                                                                 K,
                                                                 V,
                                                                 marker::LeafOrInternal>)
                                             -> BTreeMap<K, V> {

            match node.force() {
                Leaf(leaf) => {
                    let mut out_tree = BTreeMap {
                        root: node::Root::new_leaf(),
                        length: 0,
                    };

                    {
                        let mut out_node = match out_tree.root.as_mut().force() {
                            Leaf(leaf) => leaf,
                            Internal(_) => unreachable!(),
                        };

                        let mut in_edge = leaf.first_edge();
                        while let Ok(kv) = in_edge.right_kv() {
                            let (k, v) = kv.into_kv();
                            in_edge = kv.right_edge();

                            out_node.push(k.clone(), v.clone());
                            out_tree.length += 1;
                        }
                    }

                    out_tree
                }
                Internal(internal) => {
                    let mut out_tree = clone_subtree(internal.first_edge().descend());

                    {
                        let mut out_node = out_tree.root.push_level();
                        let mut in_edge = internal.first_edge();
                        while let Ok(kv) = in_edge.right_kv() {
                            let (k, v) = kv.into_kv();
                            in_edge = kv.right_edge();

                            let k = (*k).clone();
                            let v = (*v).clone();
                            let subtree = clone_subtree(in_edge.descend());

                            // We can't destructure subtree directly
                            // because BTreeMap implements Drop
                            let (subroot, sublength) = unsafe {
                                let root = ptr::read(&subtree.root);
                                let length = subtree.length;
                                mem::forget(subtree);
                                (root, length)
                            };

                            out_node.push(k, v, subroot);
                            out_tree.length += 1 + sublength;
                        }
                    }

                    out_tree
                }
            }
        }

        clone_subtree(self.root.as_ref())
    }
}

impl<K, Q: ?Sized> super::Recover<Q> for BTreeMap<K, ()>
    where K: Borrow<Q> + Ord,
          Q: Ord
{
    type Key = K;

    fn get(&self, key: &Q) -> Option<&K> {
        match search::search_tree(self.root.as_ref(), key) {
            Found(handle) => Some(handle.into_kv().0),
            GoDown(_) => None,
        }
    }

    fn take(&mut self, key: &Q) -> Option<K> {
        match search::search_tree(self.root.as_mut(), key) {
            Found(handle) => {
                Some(OccupiedEntry {
                         handle,
                         length: &mut self.length,
                         _marker: PhantomData,
                     }
                     .remove_kv()
                     .0)
            }
            GoDown(_) => None,
        }
    }

    fn replace(&mut self, key: K) -> Option<K> {
        match search::search_tree::<marker::Mut, K, (), K>(self.root.as_mut(), &key) {
            Found(handle) => Some(mem::replace(handle.into_kv_mut().0, key)),
            GoDown(handle) => {
                VacantEntry {
                    key,
                    handle,
                    length: &mut self.length,
                    _marker: PhantomData,
                }
                .insert(());
                None
            }
        }
    }
}

/// An iterator over the entries of a `BTreeMap`.
///
/// This `struct` is created by the [`iter`] method on [`BTreeMap`]. See its
/// documentation for more.
///
/// [`iter`]: struct.BTreeMap.html#method.iter
/// [`BTreeMap`]: struct.BTreeMap.html
#[stable(feature = "rust1", since = "1.0.0")]
pub struct Iter<'a, K: 'a, V: 'a> {
    range: Range<'a, K, V>,
    length: usize,
}

#[stable(feature = "collection_debug", since = "1.17.0")]
impl<'a, K: 'a + fmt::Debug, V: 'a + fmt::Debug> fmt::Debug for Iter<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// A mutable iterator over the entries of a `BTreeMap`.
///
/// This `struct` is created by the [`iter_mut`] method on [`BTreeMap`]. See its
/// documentation for more.
///
/// [`iter_mut`]: struct.BTreeMap.html#method.iter_mut
/// [`BTreeMap`]: struct.BTreeMap.html
#[stable(feature = "rust1", since = "1.0.0")]
#[derive(Debug)]
pub struct IterMut<'a, K: 'a, V: 'a> {
    range: RangeMut<'a, K, V>,
    length: usize,
}

/// An owning iterator over the entries of a `BTreeMap`.
///
/// This `struct` is created by the [`into_iter`] method on [`BTreeMap`][`BTreeMap`]
/// (provided by the `IntoIterator` trait). See its documentation for more.
///
/// [`into_iter`]: struct.BTreeMap.html#method.into_iter
/// [`BTreeMap`]: struct.BTreeMap.html
#[stable(feature = "rust1", since = "1.0.0")]
pub struct IntoIter<K, V> {
    front: Handle<NodeRef<marker::Owned, K, V, marker::Leaf>, marker::Edge>,
    back: Handle<NodeRef<marker::Owned, K, V, marker::Leaf>, marker::Edge>,
    length: usize,
}

#[stable(feature = "collection_debug", since = "1.17.0")]
impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for IntoIter<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let range = Range {
            front: self.front.reborrow(),
            back: self.back.reborrow(),
        };
        f.debug_list().entries(range).finish()
    }
}

/// An iterator over the keys of a `BTreeMap`.
///
/// This `struct` is created by the [`keys`] method on [`BTreeMap`]. See its
/// documentation for more.
///
/// [`keys`]: struct.BTreeMap.html#method.keys
/// [`BTreeMap`]: struct.BTreeMap.html
#[stable(feature = "rust1", since = "1.0.0")]
pub struct Keys<'a, K: 'a, V: 'a> {
    inner: Iter<'a, K, V>,
}

#[stable(feature = "collection_debug", since = "1.17.0")]
impl<'a, K: 'a + fmt::Debug, V: 'a> fmt::Debug for Keys<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// An iterator over the values of a `BTreeMap`.
///
/// This `struct` is created by the [`values`] method on [`BTreeMap`]. See its
/// documentation for more.
///
/// [`values`]: struct.BTreeMap.html#method.values
/// [`BTreeMap`]: struct.BTreeMap.html
#[stable(feature = "rust1", since = "1.0.0")]
pub struct Values<'a, K: 'a, V: 'a> {
    inner: Iter<'a, K, V>,
}

#[stable(feature = "collection_debug", since = "1.17.0")]
impl<'a, K: 'a, V: 'a + fmt::Debug> fmt::Debug for Values<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// A mutable iterator over the values of a `BTreeMap`.
///
/// This `struct` is created by the [`values_mut`] method on [`BTreeMap`]. See its
/// documentation for more.
///
/// [`values_mut`]: struct.BTreeMap.html#method.values_mut
/// [`BTreeMap`]: struct.BTreeMap.html
#[stable(feature = "map_values_mut", since = "1.10.0")]
#[derive(Debug)]
pub struct ValuesMut<'a, K: 'a, V: 'a> {
    inner: IterMut<'a, K, V>,
}

/// An iterator over a sub-range of entries in a `BTreeMap`.
///
/// This `struct` is created by the [`range`] method on [`BTreeMap`]. See its
/// documentation for more.
///
/// [`range`]: struct.BTreeMap.html#method.range
/// [`BTreeMap`]: struct.BTreeMap.html
#[stable(feature = "btree_range", since = "1.17.0")]
pub struct Range<'a, K: 'a, V: 'a> {
    front: Handle<NodeRef<marker::Immut<'a>, K, V, marker::Leaf>, marker::Edge>,
    back: Handle<NodeRef<marker::Immut<'a>, K, V, marker::Leaf>, marker::Edge>,
}

#[stable(feature = "collection_debug", since = "1.17.0")]
impl<'a, K: 'a + fmt::Debug, V: 'a + fmt::Debug> fmt::Debug for Range<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// A mutable iterator over a sub-range of entries in a `BTreeMap`.
///
/// This `struct` is created by the [`range_mut`] method on [`BTreeMap`]. See its
/// documentation for more.
///
/// [`range_mut`]: struct.BTreeMap.html#method.range_mut
/// [`BTreeMap`]: struct.BTreeMap.html
#[stable(feature = "btree_range", since = "1.17.0")]
pub struct RangeMut<'a, K: 'a, V: 'a> {
    front: Handle<NodeRef<marker::Mut<'a>, K, V, marker::Leaf>, marker::Edge>,
    back: Handle<NodeRef<marker::Mut<'a>, K, V, marker::Leaf>, marker::Edge>,

    // Be invariant in `K` and `V`
    _marker: PhantomData<&'a mut (K, V)>,
}

#[stable(feature = "collection_debug", since = "1.17.0")]
impl<'a, K: 'a + fmt::Debug, V: 'a + fmt::Debug> fmt::Debug for RangeMut<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let range = Range {
            front: self.front.reborrow(),
            back: self.back.reborrow(),
        };
        f.debug_list().entries(range).finish()
    }
}

/// A view into a single entry in a map, which may either be vacant or occupied.
///
/// This `enum` is constructed from the [`entry`] method on [`BTreeMap`].
///
/// [`BTreeMap`]: struct.BTreeMap.html
/// [`entry`]: struct.BTreeMap.html#method.entry
#[stable(feature = "rust1", since = "1.0.0")]
pub enum Entry<'a, K: 'a, V: 'a> {
    /// A vacant entry.
    #[stable(feature = "rust1", since = "1.0.0")]
    Vacant(#[stable(feature = "rust1", since = "1.0.0")]
           VacantEntry<'a, K, V>),

    /// An occupied entry.
    #[stable(feature = "rust1", since = "1.0.0")]
    Occupied(#[stable(feature = "rust1", since = "1.0.0")]
             OccupiedEntry<'a, K, V>),
}

#[stable(feature= "debug_btree_map", since = "1.12.0")]
impl<'a, K: 'a + Debug + Ord, V: 'a + Debug> Debug for Entry<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Vacant(ref v) => f.debug_tuple("Entry")
                              .field(v)
                              .finish(),
            Occupied(ref o) => f.debug_tuple("Entry")
                                .field(o)
                                .finish(),
        }
    }
}

/// A view into a vacant entry in a `BTreeMap`.
/// It is part of the [`Entry`] enum.
///
/// [`Entry`]: enum.Entry.html
#[stable(feature = "rust1", since = "1.0.0")]
pub struct VacantEntry<'a, K: 'a, V: 'a> {
    key: K,
    handle: Handle<NodeRef<marker::Mut<'a>, K, V, marker::Leaf>, marker::Edge>,
    length: &'a mut usize,

    // Be invariant in `K` and `V`
    _marker: PhantomData<&'a mut (K, V)>,
}

#[stable(feature= "debug_btree_map", since = "1.12.0")]
impl<'a, K: 'a + Debug + Ord, V: 'a> Debug for VacantEntry<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple("VacantEntry")
         .field(self.key())
         .finish()
    }
}

/// A view into an occupied entry in a `BTreeMap`.
/// It is part of the [`Entry`] enum.
///
/// [`Entry`]: enum.Entry.html
#[stable(feature = "rust1", since = "1.0.0")]
pub struct OccupiedEntry<'a, K: 'a, V: 'a> {
    handle: Handle<NodeRef<marker::Mut<'a>, K, V, marker::LeafOrInternal>, marker::KV>,

    length: &'a mut usize,

    // Be invariant in `K` and `V`
    _marker: PhantomData<&'a mut (K, V)>,
}

#[stable(feature= "debug_btree_map", since = "1.12.0")]
impl<'a, K: 'a + Debug + Ord, V: 'a + Debug> Debug for OccupiedEntry<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("OccupiedEntry")
         .field("key", self.key())
         .field("value", self.get())
         .finish()
    }
}

// An iterator for merging two sorted sequences into one
struct MergeIter<K, V, I: Iterator<Item = (K, V)>> {
    left: Peekable<I>,
    right: Peekable<I>,
}

impl<K: Ord, V> BTreeMap<K, V> {
    /// Makes a new empty BTreeMap with a reasonable choice for B.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map = BTreeMap::new();
    ///
    /// // entries can now be inserted into the empty map
    /// map.insert(1, "a");
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn new() -> BTreeMap<K, V> {
        BTreeMap {
            root: node::Root::new_leaf(),
            length: 0,
        }
    }

    /// Clears the map, removing all values.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut a = BTreeMap::new();
    /// a.insert(1, "a");
    /// a.clear();
    /// assert!(a.is_empty());
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn clear(&mut self) {
        // FIXME(gereeter) .clear() allocates
        *self = BTreeMap::new();
    }

    /// Returns a reference to the value corresponding to the key.
    ///
    /// The key may be any borrowed form of the map's key type, but the ordering
    /// on the borrowed form *must* match the ordering on the key type.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map = BTreeMap::new();
    /// map.insert(1, "a");
    /// assert_eq!(map.get(&1), Some(&"a"));
    /// assert_eq!(map.get(&2), None);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<&V>
        where K: Borrow<Q>,
              Q: Ord
    {
        match search::search_tree(self.root.as_ref(), key) {
            Found(handle) => Some(handle.into_kv().1),
            GoDown(_) => None,
        }
    }

    /// Returns `true` if the map contains a value for the specified key.
    ///
    /// The key may be any borrowed form of the map's key type, but the ordering
    /// on the borrowed form *must* match the ordering on the key type.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map = BTreeMap::new();
    /// map.insert(1, "a");
    /// assert_eq!(map.contains_key(&1), true);
    /// assert_eq!(map.contains_key(&2), false);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn contains_key<Q: ?Sized>(&self, key: &Q) -> bool
        where K: Borrow<Q>,
              Q: Ord
    {
        self.get(key).is_some()
    }

    /// Returns a mutable reference to the value corresponding to the key.
    ///
    /// The key may be any borrowed form of the map's key type, but the ordering
    /// on the borrowed form *must* match the ordering on the key type.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map = BTreeMap::new();
    /// map.insert(1, "a");
    /// if let Some(x) = map.get_mut(&1) {
    ///     *x = "b";
    /// }
    /// assert_eq!(map[&1], "b");
    /// ```
    // See `get` for implementation notes, this is basically a copy-paste with mut's added
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn get_mut<Q: ?Sized>(&mut self, key: &Q) -> Option<&mut V>
        where K: Borrow<Q>,
              Q: Ord
    {
        match search::search_tree(self.root.as_mut(), key) {
            Found(handle) => Some(handle.into_kv_mut().1),
            GoDown(_) => None,
        }
    }

    /// Inserts a key-value pair into the map.
    ///
    /// If the map did not have this key present, `None` is returned.
    ///
    /// If the map did have this key present, the value is updated, and the old
    /// value is returned. The key is not updated, though; this matters for
    /// types that can be `==` without being identical. See the [module-level
    /// documentation] for more.
    ///
    /// [module-level documentation]: index.html#insert-and-complex-keys
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map = BTreeMap::new();
    /// assert_eq!(map.insert(37, "a"), None);
    /// assert_eq!(map.is_empty(), false);
    ///
    /// map.insert(37, "b");
    /// assert_eq!(map.insert(37, "c"), Some("b"));
    /// assert_eq!(map[&37], "c");
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.entry(key) {
            Occupied(mut entry) => Some(entry.insert(value)),
            Vacant(entry) => {
                entry.insert(value);
                None
            }
        }
    }

    /// Removes a key from the map, returning the value at the key if the key
    /// was previously in the map.
    ///
    /// The key may be any borrowed form of the map's key type, but the ordering
    /// on the borrowed form *must* match the ordering on the key type.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map = BTreeMap::new();
    /// map.insert(1, "a");
    /// assert_eq!(map.remove(&1), Some("a"));
    /// assert_eq!(map.remove(&1), None);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
        where K: Borrow<Q>,
              Q: Ord
    {
        match search::search_tree(self.root.as_mut(), key) {
            Found(handle) => {
                Some(OccupiedEntry {
                         handle,
                         length: &mut self.length,
                         _marker: PhantomData,
                     }
                     .remove())
            }
            GoDown(_) => None,
        }
    }

    /// Moves all elements from `other` into `Self`, leaving `other` empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut a = BTreeMap::new();
    /// a.insert(1, "a");
    /// a.insert(2, "b");
    /// a.insert(3, "c");
    ///
    /// let mut b = BTreeMap::new();
    /// b.insert(3, "d");
    /// b.insert(4, "e");
    /// b.insert(5, "f");
    ///
    /// a.append(&mut b);
    ///
    /// assert_eq!(a.len(), 5);
    /// assert_eq!(b.len(), 0);
    ///
    /// assert_eq!(a[&1], "a");
    /// assert_eq!(a[&2], "b");
    /// assert_eq!(a[&3], "d");
    /// assert_eq!(a[&4], "e");
    /// assert_eq!(a[&5], "f");
    /// ```
    #[stable(feature = "btree_append", since = "1.11.0")]
    pub fn append(&mut self, other: &mut Self) {
        // Do we have to append anything at all?
        if other.len() == 0 {
            return;
        }

        // We can just swap `self` and `other` if `self` is empty.
        if self.len() == 0 {
            mem::swap(self, other);
            return;
        }

        // First, we merge `self` and `other` into a sorted sequence in linear time.
        let self_iter = mem::replace(self, BTreeMap::new()).into_iter();
        let other_iter = mem::replace(other, BTreeMap::new()).into_iter();
        let iter = MergeIter {
            left: self_iter.peekable(),
            right: other_iter.peekable(),
        };

        // Second, we build a tree from the sorted sequence in linear time.
        self.from_sorted_iter(iter);
        self.fix_right_edge();
    }

    /// Constructs a double-ended iterator over a sub-range of elements in the map.
    /// The simplest way is to use the range syntax `min..max`, thus `range(min..max)` will
    /// yield elements from min (inclusive) to max (exclusive).
    /// The range may also be entered as `(Bound<T>, Bound<T>)`, so for example
    /// `range((Excluded(4), Included(10)))` will yield a left-exclusive, right-inclusive
    /// range from 4 to 10.
    ///
    /// # Panics
    ///
    /// Panics if range `start > end`.
    /// Panics if range `start == end` and both bounds are `Excluded`.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::Bound::Included;
    ///
    /// let mut map = BTreeMap::new();
    /// map.insert(3, "a");
    /// map.insert(5, "b");
    /// map.insert(8, "c");
    /// for (&key, &value) in map.range((Included(&4), Included(&8))) {
    ///     println!("{}: {}", key, value);
    /// }
    /// assert_eq!(Some((&5, &"b")), map.range(4..).next());
    /// ```
    #[stable(feature = "btree_range", since = "1.17.0")]
    pub fn range<T: ?Sized, R>(&self, range: R) -> Range<K, V>
        where T: Ord, K: Borrow<T>, R: RangeArgument<T>
    {
        let root1 = self.root.as_ref();
        let root2 = self.root.as_ref();
        let (f, b) = range_search(root1, root2, range);

        Range { front: f, back: b}
    }

    /// Constructs a mutable double-ended iterator over a sub-range of elements in the map.
    /// The simplest way is to use the range syntax `min..max`, thus `range(min..max)` will
    /// yield elements from min (inclusive) to max (exclusive).
    /// The range may also be entered as `(Bound<T>, Bound<T>)`, so for example
    /// `range((Excluded(4), Included(10)))` will yield a left-exclusive, right-inclusive
    /// range from 4 to 10.
    ///
    /// # Panics
    ///
    /// Panics if range `start > end`.
    /// Panics if range `start == end` and both bounds are `Excluded`.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, i32> = ["Alice", "Bob", "Carol", "Cheryl"].iter()
    ///                                                                       .map(|&s| (s, 0))
    ///                                                                       .collect();
    /// for (_, balance) in map.range_mut("B".."Cheryl") {
    ///     *balance += 100;
    /// }
    /// for (name, balance) in &map {
    ///     println!("{} => {}", name, balance);
    /// }
    /// ```
    #[stable(feature = "btree_range", since = "1.17.0")]
    pub fn range_mut<T: ?Sized, R>(&mut self, range: R) -> RangeMut<K, V>
        where T: Ord, K: Borrow<T>, R: RangeArgument<T>
    {
        let root1 = self.root.as_mut();
        let root2 = unsafe { ptr::read(&root1) };
        let (f, b) = range_search(root1, root2, range);

        RangeMut {
            front: f,
            back: b,
            _marker: PhantomData,
        }
    }

    /// Gets the given key's corresponding entry in the map for in-place manipulation.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    ///
    /// // count the number of occurrences of letters in the vec
    /// for x in vec!["a","b","a","c","a","b"] {
    ///     *count.entry(x).or_insert(0) += 1;
    /// }
    ///
    /// assert_eq!(count["a"], 3);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn entry(&mut self, key: K) -> Entry<K, V> {
        match search::search_tree(self.root.as_mut(), &key) {
            Found(handle) => {
                Occupied(OccupiedEntry {
                    handle,
                    length: &mut self.length,
                    _marker: PhantomData,
                })
            }
            GoDown(handle) => {
                Vacant(VacantEntry {
                    key,
                    handle,
                    length: &mut self.length,
                    _marker: PhantomData,
                })
            }
        }
    }

    fn from_sorted_iter<I: Iterator<Item = (K, V)>>(&mut self, iter: I) {
        let mut cur_node = last_leaf_edge(self.root.as_mut()).into_node();
        // Iterate through all key-value pairs, pushing them into nodes at the right level.
        for (key, value) in iter {
            // Try to push key-value pair into the current leaf node.
            if cur_node.len() < node::CAPACITY {
                cur_node.push(key, value);
            } else {
                // No space left, go up and push there.
                let mut open_node;
                let mut test_node = cur_node.forget_type();
                loop {
                    match test_node.ascend() {
                        Ok(parent) => {
                            let parent = parent.into_node();
                            if parent.len() < node::CAPACITY {
                                // Found a node with space left, push here.
                                open_node = parent;
                                break;
                            } else {
                                // Go up again.
                                test_node = parent.forget_type();
                            }
                        }
                        Err(node) => {
                            // We are at the top, create a new root node and push there.
                            open_node = node.into_root_mut().push_level();
                            break;
                        }
                    }
                }

                // Push key-value pair and new right subtree.
                let tree_height = open_node.height() - 1;
                let mut right_tree = node::Root::new_leaf();
                for _ in 0..tree_height {
                    right_tree.push_level();
                }
                open_node.push(key, value, right_tree);

                // Go down to the right-most leaf again.
                cur_node = last_leaf_edge(open_node.forget_type()).into_node();
            }

            self.length += 1;
        }
    }

    fn fix_right_edge(&mut self) {
        // Handle underfull nodes, start from the top.
        let mut cur_node = self.root.as_mut();
        while let Internal(internal) = cur_node.force() {
            // Check if right-most child is underfull.
            let mut last_edge = internal.last_edge();
            let right_child_len = last_edge.reborrow().descend().len();
            if right_child_len < node::MIN_LEN {
                // We need to steal.
                let mut last_kv = match last_edge.left_kv() {
                    Ok(left) => left,
                    Err(_) => unreachable!(),
                };
                last_kv.bulk_steal_left(node::MIN_LEN - right_child_len);
                last_edge = last_kv.right_edge();
            }

            // Go further down.
            cur_node = last_edge.descend();
        }
    }

    /// Splits the collection into two at the given key. Returns everything after the given key,
    /// including the key.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut a = BTreeMap::new();
    /// a.insert(1, "a");
    /// a.insert(2, "b");
    /// a.insert(3, "c");
    /// a.insert(17, "d");
    /// a.insert(41, "e");
    ///
    /// let b = a.split_off(&3);
    ///
    /// assert_eq!(a.len(), 2);
    /// assert_eq!(b.len(), 3);
    ///
    /// assert_eq!(a[&1], "a");
    /// assert_eq!(a[&2], "b");
    ///
    /// assert_eq!(b[&3], "c");
    /// assert_eq!(b[&17], "d");
    /// assert_eq!(b[&41], "e");
    /// ```
    #[stable(feature = "btree_split_off", since = "1.11.0")]
    pub fn split_off<Q: ?Sized + Ord>(&mut self, key: &Q) -> Self
        where K: Borrow<Q>
    {
        if self.is_empty() {
            return Self::new();
        }

        let total_num = self.len();

        let mut right = Self::new();
        for _ in 0..(self.root.as_ref().height()) {
            right.root.push_level();
        }

        {
            let mut left_node = self.root.as_mut();
            let mut right_node = right.root.as_mut();

            loop {
                let mut split_edge = match search::search_node(left_node, key) {
                    // key is going to the right tree
                    Found(handle) => handle.left_edge(),
                    GoDown(handle) => handle,
                };

                split_edge.move_suffix(&mut right_node);

                match (split_edge.force(), right_node.force()) {
                    (Internal(edge), Internal(node)) => {
                        left_node = edge.descend();
                        right_node = node.first_edge().descend();
                    }
                    (Leaf(_), Leaf(_)) => {
                        break;
                    }
                    _ => {
                        unreachable!();
                    }
                }
            }
        }

        self.fix_right_border();
        right.fix_left_border();

        if self.root.as_ref().height() < right.root.as_ref().height() {
            self.recalc_length();
            right.length = total_num - self.len();
        } else {
            right.recalc_length();
            self.length = total_num - right.len();
        }

        right
    }

    /// Calculates the number of elements if it is incorrect.
    fn recalc_length(&mut self) {
        fn dfs<K, V>(node: NodeRef<marker::Immut, K, V, marker::LeafOrInternal>) -> usize {
            let mut res = node.len();

            if let Internal(node) = node.force() {
                let mut edge = node.first_edge();
                loop {
                    res += dfs(edge.reborrow().descend());
                    match edge.right_kv() {
                        Ok(right_kv) => {
                            edge = right_kv.right_edge();
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
            }

            res
        }

        self.length = dfs(self.root.as_ref());
    }

    /// Removes empty levels on the top.
    fn fix_top(&mut self) {
        loop {
            {
                let node = self.root.as_ref();
                if node.height() == 0 || node.len() > 0 {
                    break;
                }
            }
            self.root.pop_level();
        }
    }

    fn fix_right_border(&mut self) {
        self.fix_top();

        {
            let mut cur_node = self.root.as_mut();

            while let Internal(node) = cur_node.force() {
                let mut last_kv = node.last_kv();

                if last_kv.can_merge() {
                    cur_node = last_kv.merge().descend();
                } else {
                    let right_len = last_kv.reborrow().right_edge().descend().len();
                    // `MINLEN + 1` to avoid readjust if merge happens on the next level.
                    if right_len < node::MIN_LEN + 1 {
                        last_kv.bulk_steal_left(node::MIN_LEN + 1 - right_len);
                    }
                    cur_node = last_kv.right_edge().descend();
                }
            }
        }

        self.fix_top();
    }

    /// The symmetric clone of `fix_right_border`.
    fn fix_left_border(&mut self) {
        self.fix_top();

        {
            let mut cur_node = self.root.as_mut();

            while let Internal(node) = cur_node.force() {
                let mut first_kv = node.first_kv();

                if first_kv.can_merge() {
                    cur_node = first_kv.merge().descend();
                } else {
                    let left_len = first_kv.reborrow().left_edge().descend().len();
                    if left_len < node::MIN_LEN + 1 {
                        first_kv.bulk_steal_right(node::MIN_LEN + 1 - left_len);
                    }
                    cur_node = first_kv.left_edge().descend();
                }
            }
        }

        self.fix_top();
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K: 'a, V: 'a> IntoIterator for &'a BTreeMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    fn into_iter(self) -> Iter<'a, K, V> {
        self.iter()
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K: 'a, V: 'a> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<(&'a K, &'a V)> {
        if self.length == 0 {
            None
        } else {
            self.length -= 1;
            unsafe { Some(self.range.next_unchecked()) }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.length, Some(self.length))
    }
}

#[stable(feature = "fused", since = "1.26.0")]
impl<'a, K, V> FusedIterator for Iter<'a, K, V> {}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K: 'a, V: 'a> DoubleEndedIterator for Iter<'a, K, V> {
    fn next_back(&mut self) -> Option<(&'a K, &'a V)> {
        if self.length == 0 {
            None
        } else {
            self.length -= 1;
            unsafe { Some(self.range.next_back_unchecked()) }
        }
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K: 'a, V: 'a> ExactSizeIterator for Iter<'a, K, V> {
    fn len(&self) -> usize {
        self.length
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K, V> Clone for Iter<'a, K, V> {
    fn clone(&self) -> Iter<'a, K, V> {
        Iter {
            range: self.range.clone(),
            length: self.length,
        }
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K: 'a, V: 'a> IntoIterator for &'a mut BTreeMap<K, V> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V>;

    fn into_iter(self) -> IterMut<'a, K, V> {
        self.iter_mut()
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K: 'a, V: 'a> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<(&'a K, &'a mut V)> {
        if self.length == 0 {
            None
        } else {
            self.length -= 1;
            unsafe { Some(self.range.next_unchecked()) }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.length, Some(self.length))
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K: 'a, V: 'a> DoubleEndedIterator for IterMut<'a, K, V> {
    fn next_back(&mut self) -> Option<(&'a K, &'a mut V)> {
        if self.length == 0 {
            None
        } else {
            self.length -= 1;
            unsafe { Some(self.range.next_back_unchecked()) }
        }
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K: 'a, V: 'a> ExactSizeIterator for IterMut<'a, K, V> {
    fn len(&self) -> usize {
        self.length
    }
}

#[stable(feature = "fused", since = "1.26.0")]
impl<'a, K, V> FusedIterator for IterMut<'a, K, V> {}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K, V> IntoIterator for BTreeMap<K, V> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;

    fn into_iter(self) -> IntoIter<K, V> {
        let root1 = unsafe { ptr::read(&self.root).into_ref() };
        let root2 = unsafe { ptr::read(&self.root).into_ref() };
        let len = self.length;
        mem::forget(self);

        IntoIter {
            front: first_leaf_edge(root1),
            back: last_leaf_edge(root2),
            length: len,
        }
    }
}

#[stable(feature = "btree_drop", since = "1.7.0")]
impl<K, V> Drop for IntoIter<K, V> {
    fn drop(&mut self) {
        for _ in &mut *self {
        }
        unsafe {
            let leaf_node = ptr::read(&self.front).into_node();
            if let Some(first_parent) = leaf_node.deallocate_and_ascend() {
                let mut cur_node = first_parent.into_node();
                while let Some(parent) = cur_node.deallocate_and_ascend() {
                    cur_node = parent.into_node()
                }
            }
        }
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K, V> Iterator for IntoIter<K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<(K, V)> {
        if self.length == 0 {
            return None;
        } else {
            self.length -= 1;
        }

        let handle = unsafe { ptr::read(&self.front) };

        let mut cur_handle = match handle.right_kv() {
            Ok(kv) => {
                let k = unsafe { ptr::read(kv.reborrow().into_kv().0) };
                let v = unsafe { ptr::read(kv.reborrow().into_kv().1) };
                self.front = kv.right_edge();
                return Some((k, v));
            }
            Err(last_edge) => unsafe {
                unwrap_unchecked(last_edge.into_node().deallocate_and_ascend())
            },
        };

        loop {
            match cur_handle.right_kv() {
                Ok(kv) => {
                    let k = unsafe { ptr::read(kv.reborrow().into_kv().0) };
                    let v = unsafe { ptr::read(kv.reborrow().into_kv().1) };
                    self.front = first_leaf_edge(kv.right_edge().descend());
                    return Some((k, v));
                }
                Err(last_edge) => unsafe {
                    cur_handle = unwrap_unchecked(last_edge.into_node().deallocate_and_ascend());
                },
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.length, Some(self.length))
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K, V> DoubleEndedIterator for IntoIter<K, V> {
    fn next_back(&mut self) -> Option<(K, V)> {
        if self.length == 0 {
            return None;
        } else {
            self.length -= 1;
        }

        let handle = unsafe { ptr::read(&self.back) };

        let mut cur_handle = match handle.left_kv() {
            Ok(kv) => {
                let k = unsafe { ptr::read(kv.reborrow().into_kv().0) };
                let v = unsafe { ptr::read(kv.reborrow().into_kv().1) };
                self.back = kv.left_edge();
                return Some((k, v));
            }
            Err(last_edge) => unsafe {
                unwrap_unchecked(last_edge.into_node().deallocate_and_ascend())
            },
        };

        loop {
            match cur_handle.left_kv() {
                Ok(kv) => {
                    let k = unsafe { ptr::read(kv.reborrow().into_kv().0) };
                    let v = unsafe { ptr::read(kv.reborrow().into_kv().1) };
                    self.back = last_leaf_edge(kv.left_edge().descend());
                    return Some((k, v));
                }
                Err(last_edge) => unsafe {
                    cur_handle = unwrap_unchecked(last_edge.into_node().deallocate_and_ascend());
                },
            }
        }
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K, V> ExactSizeIterator for IntoIter<K, V> {
    fn len(&self) -> usize {
        self.length
    }
}

#[stable(feature = "fused", since = "1.26.0")]
impl<K, V> FusedIterator for IntoIter<K, V> {}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K, V> Iterator for Keys<'a, K, V> {
    type Item = &'a K;

    fn next(&mut self) -> Option<&'a K> {
        self.inner.next().map(|(k, _)| k)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K, V> DoubleEndedIterator for Keys<'a, K, V> {
    fn next_back(&mut self) -> Option<&'a K> {
        self.inner.next_back().map(|(k, _)| k)
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K, V> ExactSizeIterator for Keys<'a, K, V> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

#[stable(feature = "fused", since = "1.26.0")]
impl<'a, K, V> FusedIterator for Keys<'a, K, V> {}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K, V> Clone for Keys<'a, K, V> {
    fn clone(&self) -> Keys<'a, K, V> {
        Keys { inner: self.inner.clone() }
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K, V> Iterator for Values<'a, K, V> {
    type Item = &'a V;

    fn next(&mut self) -> Option<&'a V> {
        self.inner.next().map(|(_, v)| v)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K, V> DoubleEndedIterator for Values<'a, K, V> {
    fn next_back(&mut self) -> Option<&'a V> {
        self.inner.next_back().map(|(_, v)| v)
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K, V> ExactSizeIterator for Values<'a, K, V> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

#[stable(feature = "fused", since = "1.26.0")]
impl<'a, K, V> FusedIterator for Values<'a, K, V> {}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K, V> Clone for Values<'a, K, V> {
    fn clone(&self) -> Values<'a, K, V> {
        Values { inner: self.inner.clone() }
    }
}

#[stable(feature = "btree_range", since = "1.17.0")]
impl<'a, K, V> Iterator for Range<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<(&'a K, &'a V)> {
        if self.front == self.back {
            None
        } else {
            unsafe { Some(self.next_unchecked()) }
        }
    }
}

#[stable(feature = "map_values_mut", since = "1.10.0")]
impl<'a, K, V> Iterator for ValuesMut<'a, K, V> {
    type Item = &'a mut V;

    fn next(&mut self) -> Option<&'a mut V> {
        self.inner.next().map(|(_, v)| v)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

#[stable(feature = "map_values_mut", since = "1.10.0")]
impl<'a, K, V> DoubleEndedIterator for ValuesMut<'a, K, V> {
    fn next_back(&mut self) -> Option<&'a mut V> {
        self.inner.next_back().map(|(_, v)| v)
    }
}

#[stable(feature = "map_values_mut", since = "1.10.0")]
impl<'a, K, V> ExactSizeIterator for ValuesMut<'a, K, V> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

#[stable(feature = "fused", since = "1.26.0")]
impl<'a, K, V> FusedIterator for ValuesMut<'a, K, V> {}


impl<'a, K, V> Range<'a, K, V> {
    unsafe fn next_unchecked(&mut self) -> (&'a K, &'a V) {
        let handle = self.front;

        let mut cur_handle = match handle.right_kv() {
            Ok(kv) => {
                let ret = kv.into_kv();
                self.front = kv.right_edge();
                return ret;
            }
            Err(last_edge) => {
                let next_level = last_edge.into_node().ascend().ok();
                unwrap_unchecked(next_level)
            }
        };

        loop {
            match cur_handle.right_kv() {
                Ok(kv) => {
                    let ret = kv.into_kv();
                    self.front = first_leaf_edge(kv.right_edge().descend());
                    return ret;
                }
                Err(last_edge) => {
                    let next_level = last_edge.into_node().ascend().ok();
                    cur_handle = unwrap_unchecked(next_level);
                }
            }
        }
    }
}

#[stable(feature = "btree_range", since = "1.17.0")]
impl<'a, K, V> DoubleEndedIterator for Range<'a, K, V> {
    fn next_back(&mut self) -> Option<(&'a K, &'a V)> {
        if self.front == self.back {
            None
        } else {
            unsafe { Some(self.next_back_unchecked()) }
        }
    }
}

impl<'a, K, V> Range<'a, K, V> {
    unsafe fn next_back_unchecked(&mut self) -> (&'a K, &'a V) {
        let handle = self.back;

        let mut cur_handle = match handle.left_kv() {
            Ok(kv) => {
                let ret = kv.into_kv();
                self.back = kv.left_edge();
                return ret;
            }
            Err(last_edge) => {
                let next_level = last_edge.into_node().ascend().ok();
                unwrap_unchecked(next_level)
            }
        };

        loop {
            match cur_handle.left_kv() {
                Ok(kv) => {
                    let ret = kv.into_kv();
                    self.back = last_leaf_edge(kv.left_edge().descend());
                    return ret;
                }
                Err(last_edge) => {
                    let next_level = last_edge.into_node().ascend().ok();
                    cur_handle = unwrap_unchecked(next_level);
                }
            }
        }
    }
}

#[stable(feature = "fused", since = "1.26.0")]
impl<'a, K, V> FusedIterator for Range<'a, K, V> {}

#[stable(feature = "btree_range", since = "1.17.0")]
impl<'a, K, V> Clone for Range<'a, K, V> {
    fn clone(&self) -> Range<'a, K, V> {
        Range {
            front: self.front,
            back: self.back,
        }
    }
}

#[stable(feature = "btree_range", since = "1.17.0")]
impl<'a, K, V> Iterator for RangeMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<(&'a K, &'a mut V)> {
        if self.front == self.back {
            None
        } else {
            unsafe { Some(self.next_unchecked()) }
        }
    }
}

impl<'a, K, V> RangeMut<'a, K, V> {
    unsafe fn next_unchecked(&mut self) -> (&'a K, &'a mut V) {
        let handle = ptr::read(&self.front);

        let mut cur_handle = match handle.right_kv() {
            Ok(kv) => {
                let (k, v) = ptr::read(&kv).into_kv_mut();
                self.front = kv.right_edge();
                return (k, v);
            }
            Err(last_edge) => {
                let next_level = last_edge.into_node().ascend().ok();
                unwrap_unchecked(next_level)
            }
        };

        loop {
            match cur_handle.right_kv() {
                Ok(kv) => {
                    let (k, v) = ptr::read(&kv).into_kv_mut();
                    self.front = first_leaf_edge(kv.right_edge().descend());
                    return (k, v);
                }
                Err(last_edge) => {
                    let next_level = last_edge.into_node().ascend().ok();
                    cur_handle = unwrap_unchecked(next_level);
                }
            }
        }
    }
}

#[stable(feature = "btree_range", since = "1.17.0")]
impl<'a, K, V> DoubleEndedIterator for RangeMut<'a, K, V> {
    fn next_back(&mut self) -> Option<(&'a K, &'a mut V)> {
        if self.front == self.back {
            None
        } else {
            unsafe { Some(self.next_back_unchecked()) }
        }
    }
}

#[stable(feature = "fused", since = "1.26.0")]
impl<'a, K, V> FusedIterator for RangeMut<'a, K, V> {}

impl<'a, K, V> RangeMut<'a, K, V> {
    unsafe fn next_back_unchecked(&mut self) -> (&'a K, &'a mut V) {
        let handle = ptr::read(&self.back);

        let mut cur_handle = match handle.left_kv() {
            Ok(kv) => {
                let (k, v) = ptr::read(&kv).into_kv_mut();
                self.back = kv.left_edge();
                return (k, v);
            }
            Err(last_edge) => {
                let next_level = last_edge.into_node().ascend().ok();
                unwrap_unchecked(next_level)
            }
        };

        loop {
            match cur_handle.left_kv() {
                Ok(kv) => {
                    let (k, v) = ptr::read(&kv).into_kv_mut();
                    self.back = last_leaf_edge(kv.left_edge().descend());
                    return (k, v);
                }
                Err(last_edge) => {
                    let next_level = last_edge.into_node().ascend().ok();
                    cur_handle = unwrap_unchecked(next_level);
                }
            }
        }
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K: Ord, V> FromIterator<(K, V)> for BTreeMap<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> BTreeMap<K, V> {
        let mut map = BTreeMap::new();
        map.extend(iter);
        map
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K: Ord, V> Extend<(K, V)> for BTreeMap<K, V> {
    #[inline]
    fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

#[stable(feature = "extend_ref", since = "1.2.0")]
impl<'a, K: Ord + Copy, V: Copy> Extend<(&'a K, &'a V)> for BTreeMap<K, V> {
    fn extend<I: IntoIterator<Item = (&'a K, &'a V)>>(&mut self, iter: I) {
        self.extend(iter.into_iter().map(|(&key, &value)| (key, value)));
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K: Hash, V: Hash> Hash for BTreeMap<K, V> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for elt in self {
            elt.hash(state);
        }
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K: Ord, V> Default for BTreeMap<K, V> {
    /// Creates an empty `BTreeMap<K, V>`.
    fn default() -> BTreeMap<K, V> {
        BTreeMap::new()
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K: PartialEq, V: PartialEq> PartialEq for BTreeMap<K, V> {
    fn eq(&self, other: &BTreeMap<K, V>) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a == b)
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K: Eq, V: Eq> Eq for BTreeMap<K, V> {}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K: PartialOrd, V: PartialOrd> PartialOrd for BTreeMap<K, V> {
    #[inline]
    fn partial_cmp(&self, other: &BTreeMap<K, V>) -> Option<Ordering> {
        self.iter().partial_cmp(other.iter())
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K: Ord, V: Ord> Ord for BTreeMap<K, V> {
    #[inline]
    fn cmp(&self, other: &BTreeMap<K, V>) -> Ordering {
        self.iter().cmp(other.iter())
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<K: Debug, V: Debug> Debug for BTreeMap<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<'a, K: Ord, Q: ?Sized, V> Index<&'a Q> for BTreeMap<K, V>
    where K: Borrow<Q>,
          Q: Ord
{
    type Output = V;

    /// Returns a reference to the value corresponding to the supplied key.
    ///
    /// # Panics
    ///
    /// Panics if the key is not present in the `BTreeMap`.
    #[inline]
    fn index(&self, key: &Q) -> &V {
        self.get(key).expect("no entry found for key")
    }
}

fn first_leaf_edge<BorrowType, K, V>
    (mut node: NodeRef<BorrowType, K, V, marker::LeafOrInternal>)
     -> Handle<NodeRef<BorrowType, K, V, marker::Leaf>, marker::Edge> {
    loop {
        match node.force() {
            Leaf(leaf) => return leaf.first_edge(),
            Internal(internal) => {
                node = internal.first_edge().descend();
            }
        }
    }
}

fn last_leaf_edge<BorrowType, K, V>
    (mut node: NodeRef<BorrowType, K, V, marker::LeafOrInternal>)
     -> Handle<NodeRef<BorrowType, K, V, marker::Leaf>, marker::Edge> {
    loop {
        match node.force() {
            Leaf(leaf) => return leaf.last_edge(),
            Internal(internal) => {
                node = internal.last_edge().descend();
            }
        }
    }
}

fn range_search<BorrowType, K, V, Q: ?Sized, R: RangeArgument<Q>>(
    root1: NodeRef<BorrowType, K, V, marker::LeafOrInternal>,
    root2: NodeRef<BorrowType, K, V, marker::LeafOrInternal>,
    range: R
)-> (Handle<NodeRef<BorrowType, K, V, marker::Leaf>, marker::Edge>,
     Handle<NodeRef<BorrowType, K, V, marker::Leaf>, marker::Edge>)
        where Q: Ord, K: Borrow<Q>
{
    match (range.start(), range.end()) {
        (Excluded(s), Excluded(e)) if s==e =>
            panic!("range start and end are equal and excluded in BTreeMap"),
        (Included(s), Included(e)) |
        (Included(s), Excluded(e)) |
        (Excluded(s), Included(e)) |
        (Excluded(s), Excluded(e)) if s>e =>
            panic!("range start is greater than range end in BTreeMap"),
        _ => {},
    };

    let mut min_node = root1;
    let mut max_node = root2;
    let mut min_found = false;
    let mut max_found = false;
    let mut diverged = false;

    loop {
        let min_edge = match (min_found, range.start()) {
            (false, Included(key)) => match search::search_linear(&min_node, key) {
                (i, true) => { min_found = true; i },
                (i, false) => i,
            },
            (false, Excluded(key)) => match search::search_linear(&min_node, key) {
                (i, true) => { min_found = true; i+1 },
                (i, false) => i,
            },
            (_, Unbounded) => 0,
            (true, Included(_)) => min_node.keys().len(),
            (true, Excluded(_)) => 0,
        };

        let max_edge = match (max_found, range.end()) {
            (false, Included(key)) => match search::search_linear(&max_node, key) {
                (i, true) => { max_found = true; i+1 },
                (i, false) => i,
            },
            (false, Excluded(key)) => match search::search_linear(&max_node, key) {
                (i, true) => { max_found = true; i },
                (i, false) => i,
            },
            (_, Unbounded) => max_node.keys().len(),
            (true, Included(_)) => 0,
            (true, Excluded(_)) => max_node.keys().len(),
        };

        if !diverged {
            if max_edge < min_edge { panic!("Ord is ill-defined in BTreeMap range") }
            if min_edge != max_edge { diverged = true; }
        }

        let front = Handle::new_edge(min_node, min_edge);
        let back = Handle::new_edge(max_node, max_edge);
        match (front.force(), back.force()) {
            (Leaf(f), Leaf(b)) => {
                return (f, b);
            },
            (Internal(min_int), Internal(max_int)) => {
                min_node = min_int.descend();
                max_node = max_int.descend();
            },
            _ => unreachable!("BTreeMap has different depths"),
        };
    }
}

#[inline(always)]
unsafe fn unwrap_unchecked<T>(val: Option<T>) -> T {
    val.unwrap_or_else(|| {
        if cfg!(debug_assertions) {
            panic!("'unchecked' unwrap on None in BTreeMap");
        } else {
            intrinsics::unreachable();
        }
    })
}

impl<K, V> BTreeMap<K, V> {
    /// Gets an iterator over the entries of the map, sorted by key.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map = BTreeMap::new();
    /// map.insert(3, "c");
    /// map.insert(2, "b");
    /// map.insert(1, "a");
    ///
    /// for (key, value) in map.iter() {
    ///     println!("{}: {}", key, value);
    /// }
    ///
    /// let (first_key, first_value) = map.iter().next().unwrap();
    /// assert_eq!((*first_key, *first_value), (1, "a"));
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn iter(&self) -> Iter<K, V> {
        Iter {
            range: Range {
                front: first_leaf_edge(self.root.as_ref()),
                back: last_leaf_edge(self.root.as_ref()),
            },
            length: self.length,
        }
    }

    /// Gets a mutable iterator over the entries of the map, sorted by key.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map = BTreeMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    /// map.insert("c", 3);
    ///
    /// // add 10 to the value if the key isn't "a"
    /// for (key, value) in map.iter_mut() {
    ///     if key != &"a" {
    ///         *value += 10;
    ///     }
    /// }
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn iter_mut(&mut self) -> IterMut<K, V> {
        let root1 = self.root.as_mut();
        let root2 = unsafe { ptr::read(&root1) };
        IterMut {
            range: RangeMut {
                front: first_leaf_edge(root1),
                back: last_leaf_edge(root2),
                _marker: PhantomData,
            },
            length: self.length,
        }
    }

    /// Gets an iterator over the keys of the map, in sorted order.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut a = BTreeMap::new();
    /// a.insert(2, "b");
    /// a.insert(1, "a");
    ///
    /// let keys: Vec<_> = a.keys().cloned().collect();
    /// assert_eq!(keys, [1, 2]);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn keys<'a>(&'a self) -> Keys<'a, K, V> {
        Keys { inner: self.iter() }
    }

    /// Gets an iterator over the values of the map, in order by key.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut a = BTreeMap::new();
    /// a.insert(1, "hello");
    /// a.insert(2, "goodbye");
    ///
    /// let values: Vec<&str> = a.values().cloned().collect();
    /// assert_eq!(values, ["hello", "goodbye"]);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn values<'a>(&'a self) -> Values<'a, K, V> {
        Values { inner: self.iter() }
    }

    /// Gets a mutable iterator over the values of the map, in order by key.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut a = BTreeMap::new();
    /// a.insert(1, String::from("hello"));
    /// a.insert(2, String::from("goodbye"));
    ///
    /// for value in a.values_mut() {
    ///     value.push_str("!");
    /// }
    ///
    /// let values: Vec<String> = a.values().cloned().collect();
    /// assert_eq!(values, [String::from("hello!"),
    ///                     String::from("goodbye!")]);
    /// ```
    #[stable(feature = "map_values_mut", since = "1.10.0")]
    pub fn values_mut(&mut self) -> ValuesMut<K, V> {
        ValuesMut { inner: self.iter_mut() }
    }

    /// Returns the number of elements in the map.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut a = BTreeMap::new();
    /// assert_eq!(a.len(), 0);
    /// a.insert(1, "a");
    /// assert_eq!(a.len(), 1);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn len(&self) -> usize {
        self.length
    }

    /// Returns `true` if the map contains no elements.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut a = BTreeMap::new();
    /// assert!(a.is_empty());
    /// a.insert(1, "a");
    /// assert!(!a.is_empty());
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<'a, K: Ord, V> Entry<'a, K, V> {
    /// Ensures a value is in the entry by inserting the default if empty, and returns
    /// a mutable reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_insert(12);
    ///
    /// assert_eq!(map["poneyland"], 12);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn or_insert(self, default: V) -> &'a mut V {
        match self {
            Occupied(entry) => entry.into_mut(),
            Vacant(entry) => entry.insert(default),
        }
    }

    /// Ensures a value is in the entry by inserting the result of the default function if empty,
    /// and returns a mutable reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, String> = BTreeMap::new();
    /// let s = "hoho".to_string();
    ///
    /// map.entry("poneyland").or_insert_with(|| s);
    ///
    /// assert_eq!(map["poneyland"], "hoho".to_string());
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
        match self {
            Occupied(entry) => entry.into_mut(),
            Vacant(entry) => entry.insert(default()),
        }
    }

    /// Returns a reference to this entry's key.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// assert_eq!(map.entry("poneyland").key(), &"poneyland");
    /// ```
    #[stable(feature = "map_entry_keys", since = "1.10.0")]
    pub fn key(&self) -> &K {
        match *self {
            Occupied(ref entry) => entry.key(),
            Vacant(ref entry) => entry.key(),
        }
    }

    /// Provides in-place mutable access to an occupied entry before any
    /// potential inserts into the map.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    ///
    /// map.entry("poneyland")
    ///    .and_modify(|e| { *e += 1 })
    ///    .or_insert(42);
    /// assert_eq!(map["poneyland"], 42);
    ///
    /// map.entry("poneyland")
    ///    .and_modify(|e| { *e += 1 })
    ///    .or_insert(42);
    /// assert_eq!(map["poneyland"], 43);
    /// ```
    #[stable(feature = "entry_and_modify", since = "1.26.0")]
    pub fn and_modify<F>(self, mut f: F) -> Self
        where F: FnMut(&mut V)
    {
        match self {
            Occupied(mut entry) => {
                f(entry.get_mut());
                Occupied(entry)
            },
            Vacant(entry) => Vacant(entry),
        }
    }
}

impl<'a, K: Ord, V: Default> Entry<'a, K, V> {
    #[unstable(feature = "entry_or_default", issue = "44324")]
    /// Ensures a value is in the entry by inserting the default value if empty,
    /// and returns a mutable reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// #![feature(entry_or_default)]
    /// # fn main() {
    /// use std::collections::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, Option<usize>> = BTreeMap::new();
    /// map.entry("poneyland").or_default();
    ///
    /// assert_eq!(map["poneyland"], None);
    /// # }
    /// ```
    pub fn or_default(self) -> &'a mut V {
        match self {
            Occupied(entry) => entry.into_mut(),
            Vacant(entry) => entry.insert(Default::default()),
        }
    }

}

impl<'a, K: Ord, V> VacantEntry<'a, K, V> {
    /// Gets a reference to the key that would be used when inserting a value
    /// through the VacantEntry.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// assert_eq!(map.entry("poneyland").key(), &"poneyland");
    /// ```
    #[stable(feature = "map_entry_keys", since = "1.10.0")]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Take ownership of the key.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    ///
    /// if let Entry::Vacant(v) = map.entry("poneyland") {
    ///     v.into_key();
    /// }
    /// ```
    #[stable(feature = "map_entry_recover_keys2", since = "1.12.0")]
    pub fn into_key(self) -> K {
        self.key
    }

    /// Sets the value of the entry with the `VacantEntry`'s key,
    /// and returns a mutable reference to it.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    ///
    /// // count the number of occurrences of letters in the vec
    /// for x in vec!["a","b","a","c","a","b"] {
    ///     *count.entry(x).or_insert(0) += 1;
    /// }
    ///
    /// assert_eq!(count["a"], 3);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn insert(self, value: V) -> &'a mut V {
        *self.length += 1;

        let out_ptr;

        let mut ins_k;
        let mut ins_v;
        let mut ins_edge;

        let mut cur_parent = match self.handle.insert(self.key, value) {
            (Fit(handle), _) => return handle.into_kv_mut().1,
            (Split(left, k, v, right), ptr) => {
                ins_k = k;
                ins_v = v;
                ins_edge = right;
                out_ptr = ptr;
                left.ascend().map_err(|n| n.into_root_mut())
            }
        };

        loop {
            match cur_parent {
                Ok(parent) => {
                    match parent.insert(ins_k, ins_v, ins_edge) {
                        Fit(_) => return unsafe { &mut *out_ptr },
                        Split(left, k, v, right) => {
                            ins_k = k;
                            ins_v = v;
                            ins_edge = right;
                            cur_parent = left.ascend().map_err(|n| n.into_root_mut());
                        }
                    }
                }
                Err(root) => {
                    root.push_level().push(ins_k, ins_v, ins_edge);
                    return unsafe { &mut *out_ptr };
                }
            }
        }
    }
}

impl<'a, K: Ord, V> OccupiedEntry<'a, K, V> {
    /// Gets a reference to the key in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_insert(12);
    /// assert_eq!(map.entry("poneyland").key(), &"poneyland");
    /// ```
    #[stable(feature = "map_entry_keys", since = "1.10.0")]
    pub fn key(&self) -> &K {
        self.handle.reborrow().into_kv().0
    }

    /// Take ownership of the key and value from the map.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_insert(12);
    ///
    /// if let Entry::Occupied(o) = map.entry("poneyland") {
    ///     // We delete the entry from the map.
    ///     o.remove_entry();
    /// }
    ///
    /// // If now try to get the value, it will panic:
    /// // println!("{}", map["poneyland"]);
    /// ```
    #[stable(feature = "map_entry_recover_keys2", since = "1.12.0")]
    pub fn remove_entry(self) -> (K, V) {
        self.remove_kv()
    }

    /// Gets a reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_insert(12);
    ///
    /// if let Entry::Occupied(o) = map.entry("poneyland") {
    ///     assert_eq!(o.get(), &12);
    /// }
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn get(&self) -> &V {
        self.handle.reborrow().into_kv().1
    }

    /// Gets a mutable reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_insert(12);
    ///
    /// assert_eq!(map["poneyland"], 12);
    /// if let Entry::Occupied(mut o) = map.entry("poneyland") {
    ///      *o.get_mut() += 10;
    /// }
    /// assert_eq!(map["poneyland"], 22);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn get_mut(&mut self) -> &mut V {
        self.handle.kv_mut().1
    }

    /// Converts the entry into a mutable reference to its value.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_insert(12);
    ///
    /// assert_eq!(map["poneyland"], 12);
    /// if let Entry::Occupied(o) = map.entry("poneyland") {
    ///     *o.into_mut() += 10;
    /// }
    /// assert_eq!(map["poneyland"], 22);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn into_mut(self) -> &'a mut V {
        self.handle.into_kv_mut().1
    }

    /// Sets the value of the entry with the `OccupiedEntry`'s key,
    /// and returns the entry's old value.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_insert(12);
    ///
    /// if let Entry::Occupied(mut o) = map.entry("poneyland") {
    ///     assert_eq!(o.insert(15), 12);
    /// }
    /// assert_eq!(map["poneyland"], 15);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn insert(&mut self, value: V) -> V {
        mem::replace(self.get_mut(), value)
    }

    /// Takes the value of the entry out of the map, and returns it.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_insert(12);
    ///
    /// if let Entry::Occupied(o) = map.entry("poneyland") {
    ///     assert_eq!(o.remove(), 12);
    /// }
    /// // If we try to get "poneyland"'s value, it'll panic:
    /// // println!("{}", map["poneyland"]);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    pub fn remove(self) -> V {
        self.remove_kv().1
    }

    fn remove_kv(self) -> (K, V) {
        *self.length -= 1;

        let (small_leaf, old_key, old_val) = match self.handle.force() {
            Leaf(leaf) => {
                let (hole, old_key, old_val) = leaf.remove();
                (hole.into_node(), old_key, old_val)
            }
            Internal(mut internal) => {
                let key_loc = internal.kv_mut().0 as *mut K;
                let val_loc = internal.kv_mut().1 as *mut V;

                let to_remove = first_leaf_edge(internal.right_edge().descend()).right_kv().ok();
                let to_remove = unsafe { unwrap_unchecked(to_remove) };

                let (hole, key, val) = to_remove.remove();

                let old_key = unsafe { mem::replace(&mut *key_loc, key) };
                let old_val = unsafe { mem::replace(&mut *val_loc, val) };

                (hole.into_node(), old_key, old_val)
            }
        };

        // Handle underflow
        let mut cur_node = small_leaf.forget_type();
        while cur_node.len() < node::CAPACITY / 2 {
            match handle_underfull_node(cur_node) {
                AtRoot => break,
                EmptyParent(_) => unreachable!(),
                Merged(parent) => {
                    if parent.len() == 0 {
                        // We must be at the root
                        parent.into_root_mut().pop_level();
                        break;
                    } else {
                        cur_node = parent.forget_type();
                    }
                }
                Stole(_) => break,
            }
        }

        (old_key, old_val)
    }
}

enum UnderflowResult<'a, K, V> {
    AtRoot,
    EmptyParent(NodeRef<marker::Mut<'a>, K, V, marker::Internal>),
    Merged(NodeRef<marker::Mut<'a>, K, V, marker::Internal>),
    Stole(NodeRef<marker::Mut<'a>, K, V, marker::Internal>),
}

fn handle_underfull_node<'a, K, V>(node: NodeRef<marker::Mut<'a>, K, V, marker::LeafOrInternal>)
                                   -> UnderflowResult<'a, K, V> {
    let parent = if let Ok(parent) = node.ascend() {
        parent
    } else {
        return AtRoot;
    };

    let (is_left, mut handle) = match parent.left_kv() {
        Ok(left) => (true, left),
        Err(parent) => {
            match parent.right_kv() {
                Ok(right) => (false, right),
                Err(parent) => {
                    return EmptyParent(parent.into_node());
                }
            }
        }
    };

    if handle.can_merge() {
        Merged(handle.merge().into_node())
    } else {
        if is_left {
            handle.steal_left();
        } else {
            handle.steal_right();
        }
        Stole(handle.into_node())
    }
}

impl<K: Ord, V, I: Iterator<Item = (K, V)>> Iterator for MergeIter<K, V, I> {
    type Item = (K, V);

    fn next(&mut self) -> Option<(K, V)> {
        let res = match (self.left.peek(), self.right.peek()) {
            (Some(&(ref left_key, _)), Some(&(ref right_key, _))) => left_key.cmp(right_key),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => return None,
        };

        // Check which elements comes first and only advance the corresponding iterator.
        // If two keys are equal, take the value from `right`.
        match res {
            Ordering::Less => self.left.next(),
            Ordering::Greater => self.right.next(),
            Ordering::Equal => {
                self.left.next();
                self.right.next()
            }
        }
    }
}
