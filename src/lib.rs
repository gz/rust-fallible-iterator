//! "Fallible" iterators.
//!
//! The iterator APIs in the rust standard library do not support iteration
//! that can fail in a particularly robust way. The way that these iterators
//! are typically modeled as iterators over `Result<T, E>`; for example, the
//! `Lines` iterator returns `io::Result<String>`s. When simply iterating over
//! these types, the value being iterated over either has be unwrapped in some
//! way before it can be used:
//!
//! ```ignore
//! for line in reader.lines() {
//!     let line = try!(line);
//!     // work with line
//! }
//! ```
//!
//! In addition, many of the additional methods on the `Iterator` trait will
//! not behave properly in the presence of errors when working with these kinds
//! of iterators. For example, if one wanted to count the number of lines of
//! text in a `Read`er, this might be a way to go about it:
//!
//! ```ignore
//! let count = reader.lines().count();
//! ```
//!
//! This will return the proper value when the reader operates successfully, but
//! if it encounters an IO error, the result will either be slightly higher than
//! expected if the error is transient, or it may run forever if the error is
//! returned repeatedly!
//!
//! In contrast, a fallible iterator is built around the concept that a call to
//! `next` can fail. The trait has an additional `Error` associated type in
//! addition to the `Item` type, and `next` returns `Result<Option<Self::Item>,
//! Self::Error>` rather than `Option<Self::Item>`. Methods like `count` return
//! `Result`s as well.
//!
//! This does mean that fallible iterators are incompatible with Rust's for loop
//! syntax, but `while let` loops offer a similar level of ergonomics:
//!
//! ```ignore
//! while let Some(item) = try!(iter.next()) {
//!     // work with item
//! }
//! ```
#![doc(html_root_url = "https://sfackler.github.io/rust-fallible-iterator/doc/v0.1.0")]

use std::cmp;
use std::collections::{HashMap, HashSet, BTreeMap, BTreeSet};
use std::iter;
use std::hash::Hash;

/// An `Iterator`-like trait that allows for calculation of items to fail.
pub trait FallibleIterator {
    /// The type being iterated over.
    type Item;

    /// The error type.
    type Error;

    /// Advances the iterator and returns the next value.
    ///
    /// Returns `Ok(None)` when iteration is finished.
    ///
    /// The behavior of calling this method after a previous call has returned
    /// `Ok(None)` or `Err` is implemenetation defined.
    fn next(&mut self) -> Result<Option<Self::Item>, Self::Error>;

    /// Returns bounds on the remaining length of the iterator.
    ///
    /// Specifically, the first half of the returned tuple is a lower bound and
    /// the second half is an upper bound.
    ///
    /// For the upper bound, `None` indicates that the upper bound is either
    /// unknown or larger than can be represented as a `usize`.
    ///
    /// Both bounds assume that all remaining calls to `next` succeed. That is,
    /// `next` could return an `Err` in fewer calls than specified by the lower
    /// bound.
    ///
    /// The default implementation returns `(0, None)`, which is correct for
    /// any iterator.
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }

    /// Determines if all elements of this iterator matches a predicate.
    #[inline]
    fn all<F>(&mut self, mut f: F) -> Result<bool, Self::Error>
        where F: FnMut(Self::Item) -> bool
    {
        while let Some(v) = try!(self.next()) {
            if !f(v) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Determines if any element of this iterator matches a predicate.
    #[inline]
    fn any<F>(&mut self, mut f: F) -> Result<bool, Self::Error>
        where F: FnMut(Self::Item) -> bool
    {
        while let Some(v) = try!(self.next()) {
            if f(v) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Borrow an iterator rather than consuming it.
    ///
    /// This is useful to allow the use of iterator adaptors that would
    /// otherwise consume the value.
    #[inline]
    fn by_ref(&mut self) -> &mut Self
        where Self: Sized
    {
        self
    }

    /// Returns an iterator which yields the elements of this iterator followed
    /// by another.
    #[inline]
    fn chain<I>(self, it: I) -> Chain<Self, I>
        where I: IntoFallibleIterator<Item = Self::Item, Error = Self::Error>,
              Self: Sized
    {
        Chain {
            front: self,
            back: it,
            state: ChainState::Both,
        }
    }

    /// Returns an iterator which clones all of its elements.
    #[inline]
    fn cloned<'a, T>(self) -> Cloned<Self>
        where Self: Sized + FallibleIterator<Item = &'a T>,
              T: 'a + Clone
    {
        Cloned(self)
    }

    /// Consumes the iterator, returning the number of remaining items.
    #[inline]
    fn count(mut self) -> Result<usize, Self::Error>
        where Self: Sized
    {
        let mut count = 0;
        while let Some(_) = try!(self.next()) {
            count += 1;
        }

        Ok(count)
    }

    /// Transforms the iterator into a collection.
    ///
    /// An `Err` will be returned if any invocation of `next` returns `Err`.
    #[inline]
    fn collect<T>(self) -> Result<T, Self::Error>
        where T: FromFallibleIterator<Self::Item>,
              Self: Sized
    {
        T::from_fallible_iterator(self)
    }

    /// Returns an iterator which yields the current iteration count as well
    /// as the value.
    #[inline]
    fn enumerate(self) -> Enumerate<Self>
        where Self: Sized
    {
        Enumerate { it: self, n: 0 }
    }

    /// Returns an iterator which uses a predicate to determine which values
    /// should be yielded.
    #[inline]
    fn filter<F>(self, f: F) -> Filter<Self, F>
        where Self: Sized,
              F: FnMut(&Self::Item) -> bool
    {
        Filter {
            it: self,
            f: f,
        }
    }

    /// Returns an iterator which both filters and maps.
    #[inline]
    fn filter_map<B, F>(self, f: F) -> FilterMap<Self, F>
        where Self: Sized,
              F: FnMut(Self::Item) -> Option<B>
    {
        FilterMap {
            it: self,
            f: f,
        }
    }

    /// Returns an iterator which yields this iterator's elements and ends after
    /// the frist `Ok(None)`.
    ///
    /// The behavior of calling `next` after it has previously returned
    /// `Ok(None)` is normally unspecified. The iterator returned by this method
    /// guarantees that `Ok(None)` will always be returned.
    #[inline]
    fn fuse(self) -> Fuse<Self>
        where Self: Sized
    {
        Fuse {
            it: self,
            done: false,
        }
    }

    /// Returns a normal (non-fallible) iterator over `Result<Item, Error>`.
    #[inline]
    fn iterator(self) -> Iterator<Self>
        where Self: Sized
    {
        Iterator(self)
    }

    /// Returns the last element of the iterator.
    #[inline]
    fn last(mut self) -> Result<Option<Self::Item>, Self::Error>
        where Self: Sized
    {
        let mut last = None;
        while let Some(e) = try!(self.next()) {
            last = Some(e);
        }
        Ok(last)
    }

    /// Returns an iterator which applies a transform to the elements of the
    /// underlying iterator.
    #[inline]
    fn map<B, F>(self, f: F) -> Map<Self, F>
        where F: FnMut(Self::Item) -> B,
              Self: Sized
    {
        Map { it: self, f: f }
    }

    /// Returns an iterator which applies a transform to the errors of the
    /// underlying iterator.
    #[inline]
    fn map_err<B, F>(self, f: F) -> MapErr<Self, F>
        where F: FnMut(Self::Error) -> B,
              Self: Sized
    {
        MapErr { it: self, f: f }
    }

    /// Returns the `n`th element of the iterator.
    #[inline]
    fn nth(&mut self, mut n: usize) -> Result<Option<Self::Item>, Self::Error> {
        let mut it = self.take(n);
        while let Some(e) = try!(it.next()) {
            if n == 0 {
                return Ok(Some(e));
            }
            n -= 1;
        }
        Ok(None)
    }

    /// Returns an iterator that can peek at the next element without consuming
    /// it.
    #[inline]
    fn peekable(self) -> Peekable<Self>
        where Self: Sized
    {
        Peekable {
            it: self,
            next: None,
        }
    }

    /// Returns an iterator that yields this iterator's items in the opposite
    /// order.
    #[inline]
    fn rev(self) -> Rev<Self>
        where Self: Sized + DoubleEndedFallibleIterator
    {
        Rev(self)
    }

    /// Returns an iterator that yeilds only the first `n` values of this
    /// iterator.
    #[inline]
    fn take(self, n: usize) -> Take<Self>
        where Self: Sized
    {
        Take {
            it: self,
            remaining: n,
        }
    }

    /// Returns an iterator that yields pairs of this iterator's and another
    /// iterator's values.
    #[inline]
    fn zip<I>(self, o: I) -> Zip<Self, I::IntoIter>
        where Self: Sized,
              I: IntoFallibleIterator<Error = Self::Error>
    {
        Zip(self, o.into_fallible_iterator())
    }
}

impl<'a, I: FallibleIterator + ?Sized> FallibleIterator for &'a mut I {
    type Item = I::Item;
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<I::Item>, I::Error> {
        (**self).next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }
}

impl<'a, I: DoubleEndedFallibleIterator + ?Sized> DoubleEndedFallibleIterator for &'a mut I {
    #[inline]
    fn next_back(&mut self) -> Result<Option<I::Item>, I::Error> {
        (**self).next_back()
    }
}

impl<I: FallibleIterator + ?Sized> FallibleIterator for Box<I> {
    type Item = I::Item;
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<I::Item>, I::Error> {
        (**self).next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }
}

impl<I: DoubleEndedFallibleIterator + ?Sized> DoubleEndedFallibleIterator for Box<I> {
    #[inline]
    fn next_back(&mut self) -> Result<Option<I::Item>, I::Error> {
        (**self).next_back()
    }
}

/// A fallible iterator able to yield elements from both ends.
pub trait DoubleEndedFallibleIterator: FallibleIterator {
    /// Advances the end of the iterator, returning the last value.
    fn next_back(&mut self) -> Result<Option<Self::Item>, Self::Error>;
}

pub trait FromFallibleIterator<T>: Sized {
    fn from_fallible_iterator<I>(it: I) -> Result<Self, I::Error>
        where I: FallibleIterator<Item = T>;
}

impl<T> FromFallibleIterator<T> for Vec<T> {
    #[inline]
    fn from_fallible_iterator<I>(mut it: I) -> Result<Vec<T>, I::Error>
        where I: FallibleIterator<Item = T>
    {
        let mut vec = Vec::with_capacity(it.size_hint().0);
        while let Some(v) = try!(it.next()) {
            vec.push(v);
        }
        Ok(vec)
    }
}

impl<T> FromFallibleIterator<T> for HashSet<T>
    where T: Hash + Eq
{
    #[inline]
    fn from_fallible_iterator<I>(mut it: I) -> Result<HashSet<T>, I::Error>
        where I: FallibleIterator<Item = T>
    {
        let mut set = HashSet::with_capacity(it.size_hint().0);
        while let Some(v) = try!(it.next()) {
            set.insert(v);
        }
        Ok(set)
    }
}

impl<K, V> FromFallibleIterator<(K, V)> for HashMap<K, V>
    where K: Hash + Eq
{
    #[inline]
    fn from_fallible_iterator<I>(mut it: I) -> Result<HashMap<K, V>, I::Error>
        where I: FallibleIterator<Item = (K, V)>
    {
        let mut map = HashMap::with_capacity(it.size_hint().0);
        while let Some((k, v)) = try!(it.next()) {
            map.insert(k, v);
        }
        Ok(map)
    }
}

impl<T> FromFallibleIterator<T> for BTreeSet<T>
    where T: Ord
{
    #[inline]
    fn from_fallible_iterator<I>(mut it: I) -> Result<BTreeSet<T>, I::Error>
        where I: FallibleIterator<Item = T>
    {
        let mut set = BTreeSet::new();
        while let Some(v) = try!(it.next()) {
            set.insert(v);
        }
        Ok(set)
    }
}

impl<K, V> FromFallibleIterator<(K, V)> for BTreeMap<K, V>
    where K: Ord
{
    #[inline]
    fn from_fallible_iterator<I>(mut it: I) -> Result<BTreeMap<K, V>, I::Error>
        where I: FallibleIterator<Item = (K, V)>
    {
        let mut map = BTreeMap::new();
        while let Some((k, v)) = try!(it.next()) {
            map.insert(k, v);
        }
        Ok(map)
    }
}

/// Conversion into a `FallibleIterator`.
pub trait IntoFallibleIterator {
    type Item;
    type Error;
    type IntoIter: FallibleIterator<Item = Self::Item, Error = Self::Error>;

    fn into_fallible_iterator(self) -> Self::IntoIter;
}

impl<I> IntoFallibleIterator for I
    where I: FallibleIterator
{
    type Item = I::Item;
    type Error = I::Error;
    type IntoIter = I;

    #[inline]
    fn into_fallible_iterator(self) -> I {
        self
    }
}

#[derive(Debug)]
enum ChainState {
    Both,
    Front,
    Back,
}

/// An iterator which yields the elements of one iterator followed by another.
#[derive(Debug)]
pub struct Chain<T, U> {
    front: T,
    back: U,
    state: ChainState,
}

impl<T, U> FallibleIterator for Chain<T, U>
    where T: FallibleIterator,
          U: FallibleIterator<Item = T::Item, Error = T::Error>
{
    type Item = T::Item;
    type Error = T::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<T::Item>, T::Error> {
        match self.state {
            ChainState::Both => {
                match try!(self.front.next()) {
                    Some(e) => Ok(Some(e)),
                    None => {
                        self.state = ChainState::Back;
                        self.back.next()
                    }
                }
            }
            ChainState::Front => self.front.next(),
            ChainState::Back => self.back.next(),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let front_hint = self.front.size_hint();
        let back_hint = self.back.size_hint();

        let low = front_hint.0.saturating_add(back_hint.0);
        let high = match (front_hint.1, back_hint.1) {
            (Some(f), Some(b)) => f.checked_add(b),
            _ => None,
        };

        (low, high)
    }

    #[inline]
    fn count(self) -> Result<usize, T::Error> {
        match self.state {
            ChainState::Both => Ok(try!(self.front.count()) + try!(self.back.count())),
            ChainState::Front => self.front.count(),
            ChainState::Back => self.back.count(),
        }
    }
}

impl<T, U> DoubleEndedFallibleIterator for Chain<T, U>
    where T: DoubleEndedFallibleIterator,
          U: DoubleEndedFallibleIterator<Item = T::Item, Error = T::Error>
{
    #[inline]
    fn next_back(&mut self) -> Result<Option<T::Item>, T::Error> {
        match self.state {
            ChainState::Both => {
                match try!(self.back.next_back()) {
                    Some(e) => Ok(Some(e)),
                    None => {
                        self.state = ChainState::Front;
                        self.front.next_back()
                    }
                }
            }
            ChainState::Front => self.front.next_back(),
            ChainState::Back => self.back.next_back(),
        }
    }
}

/// An iterator which clones the elements of the underlying iterator.
#[derive(Debug)]
pub struct Cloned<I>(I);

impl<'a, T, I> FallibleIterator for Cloned<I>
    where I: FallibleIterator<Item = &'a T>,
          T: 'a + Clone
{
    type Item = T;
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<T>, I::Error> {
        self.0.next().map(|o| o.cloned())
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }

    #[inline]
    fn count(self) -> Result<usize, I::Error> {
        self.0.count()
    }
}

impl<'a, T, I> DoubleEndedFallibleIterator for Cloned<I>
    where I: DoubleEndedFallibleIterator<Item = &'a T>,
          T: 'a + Clone
{
    #[inline]
    fn next_back(&mut self) -> Result<Option<T>, I::Error> {
        self.0.next_back().map(|o| o.cloned())
    }
}

/// Converts an `Iterator<Item = Result<T, E>>` into a `FallibleIterator<Item = T, Error = E>`.
#[inline]
pub fn convert<T, E, I>(it: I) -> Convert<I>
    where I: iter::Iterator<Item = Result<T, E>>
{
    Convert(it)
}

/// A fallible iterator that wraps a normal iterator over `Result`s.
#[derive(Debug)]
pub struct Convert<I>(I);

impl<T, E, I> FallibleIterator for Convert<I>
    where I: iter::Iterator<Item = Result<T, E>>
{
    type Item = T;
    type Error = E;

    #[inline]
    fn next(&mut self) -> Result<Option<T>, E> {
        match self.0.next() {
            Some(Ok(i)) => Ok(Some(i)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<T, E, I> DoubleEndedFallibleIterator for Convert<I>
    where I: DoubleEndedIterator<Item = Result<T, E>>
{
    #[inline]
    fn next_back(&mut self) -> Result<Option<T>, E> {
        match self.0.next_back() {
            Some(Ok(i)) => Ok(Some(i)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }
}

/// An iterator that yields the iteration count as well as the values of the
/// underlying iterator.
#[derive(Debug)]
pub struct Enumerate<I> {
    it: I,
    n: usize,
}

impl<I> FallibleIterator for Enumerate<I>
    where I: FallibleIterator
{
    type Item = (usize, I::Item);
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<(usize, I::Item)>, I::Error> {
        self.it
            .next()
            .map(|o| {
                o.map(|e| {
                    let i = self.n;
                    self.n += 1;
                    (i, e)
                })
            })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.it.size_hint()
    }

    #[inline]
    fn count(self) -> Result<usize, I::Error> {
        self.it.count()
    }
}

/// An iterator which uses a predicate to determine which values of the
/// underlying iterator should be yielded.
#[derive(Debug)]
pub struct Filter<I, F> {
    it: I,
    f: F,
}

impl<I, F> FallibleIterator for Filter<I, F>
    where I: FallibleIterator,
          F: FnMut(&I::Item) -> bool,
{
    type Item = I::Item;
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<I::Item>, I::Error> {
        while let Some(v) = try!(self.it.next()) {
           if (self.f)(&v) {
               return Ok(Some(v));
           }
        }

        Ok(None)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.it.size_hint().1)
    }
}

impl<I, F> DoubleEndedFallibleIterator for Filter<I, F>
    where I: DoubleEndedFallibleIterator,
          F: FnMut(&I::Item) -> bool
{
    #[inline]
    fn next_back(&mut self) -> Result<Option<I::Item>, I::Error> {
        while let Some(v) = try!(self.it.next_back()) {
           if (self.f)(&v) {
               return Ok(Some(v));
           }
        }

        Ok(None)
    }
}

/// An iterator which both filters and maps the values of the underlying
/// iterator.
#[derive(Debug)]
pub struct FilterMap<I, F> {
    it: I,
    f: F,
}

impl<B, I, F> FallibleIterator for FilterMap<I, F>
    where I: FallibleIterator,
          F: FnMut(I::Item) -> Option<B>,
{
    type Item = B;
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<B>, I::Error> {
        while let Some(v) = try!(self.it.next()) {
           if let Some(v) = (self.f)(v) {
               return Ok(Some(v));
           }
        }

        Ok(None)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.it.size_hint().1)
    }
}

impl<B, I, F> DoubleEndedFallibleIterator for FilterMap<I, F>
    where I: DoubleEndedFallibleIterator,
          F: FnMut(I::Item) -> Option<B>,
{
    #[inline]
    fn next_back(&mut self) -> Result<Option<B>, I::Error> {
        while let Some(v) = try!(self.it.next_back()) {
           if let Some(v) = (self.f)(v) {
               return Ok(Some(v));
           }
        }

        Ok(None)
    }
}

/// An iterator that yields `Ok(None)` forever after the underlying iterator
/// yields `Ok(None)` once.
#[derive(Debug)]
pub struct Fuse<I> {
    it: I,
    done: bool,
}

impl<I> FallibleIterator for Fuse<I>
    where I: FallibleIterator
{
    type Item = I::Item;
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<I::Item>, I::Error> {
        if self.done {
            return Ok(None);
        }

        match self.it.next() {
            Ok(Some(i)) => Ok(Some(i)),
            Ok(None) => {
                self.done = true;
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.it.size_hint()
    }

    #[inline]
    fn count(self) -> Result<usize, I::Error> {
        if self.done {
            Ok(0)
        } else {
            self.it.count()
        }
    }
}

/// A normal (non-fallible) iterator which wraps a fallible iterator.
#[derive(Debug)]
pub struct Iterator<I>(I);

impl<I> iter::Iterator for Iterator<I>
    where I: FallibleIterator
{
    type Item = Result<I::Item, I::Error>;

    #[inline]
    fn next(&mut self) -> Option<Result<I::Item, I::Error>> {
        match self.0.next() {
            Ok(Some(v)) => Some(Ok(v)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<I> DoubleEndedIterator for Iterator<I>
    where I: DoubleEndedFallibleIterator
{
    #[inline]
    fn next_back(&mut self) -> Option<Result<I::Item, I::Error>> {
        match self.0.next_back() {
            Ok(Some(v)) => Some(Ok(v)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

/// An iterator which applies a transform to the elements of the underlying
/// iterator.
#[derive(Debug)]
pub struct Map<I, F> {
    it: I,
    f: F,
}

impl<B, F, I> FallibleIterator for Map<I, F>
    where I: FallibleIterator,
          F: FnMut(I::Item) -> B
{
    type Item = B;
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<B>, I::Error> {
        self.it.next().map(|o| o.map(&mut self.f))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.it.size_hint()
    }

    #[inline]
    fn count(self) -> Result<usize, I::Error> {
        self.it.count()
    }
}

impl<B, F, I> DoubleEndedFallibleIterator for Map<I, F>
    where I: DoubleEndedFallibleIterator,
          F: FnMut(I::Item) -> B
{
    #[inline]
    fn next_back(&mut self) -> Result<Option<B>, I::Error> {
        self.it.next_back().map(|o| o.map(&mut self.f))
    }
}

/// An iterator which applies a transform to the errors of the underlying
/// iterator.
#[derive(Debug)]
pub struct MapErr<I, F> {
    it: I,
    f: F,
}

impl<B, F, I> FallibleIterator for MapErr<I, F>
    where I: FallibleIterator,
          F: FnMut(I::Error) -> B
{
    type Item = I::Item;
    type Error = B;

    #[inline]
    fn next(&mut self) -> Result<Option<I::Item>, B> {
        self.it.next().map_err(&mut self.f)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.it.size_hint()
    }

    #[inline]
    fn count(mut self) -> Result<usize, B> {
        self.it.count().map_err(&mut self.f)
    }
}

impl<B, F, I> DoubleEndedFallibleIterator for MapErr<I, F>
    where I: DoubleEndedFallibleIterator,
          F: FnMut(I::Error) -> B
{
    #[inline]
    fn next_back(&mut self) -> Result<Option<I::Item>, B> {
        self.it.next_back().map_err(&mut self.f)
    }
}

/// An iterator which can look at the next element without consuming it.
#[derive(Debug)]
pub struct Peekable<I: FallibleIterator> {
    it: I,
    next: Option<I::Item>,
}

impl<I> Peekable<I>
    where I: FallibleIterator
{
    /// Returns a reference to the next value without advancing the iterator.
    #[inline]
    pub fn peek(&mut self) -> Result<Option<&I::Item>, I::Error> {
        if self.next.is_none() {
            self.next = try!(self.it.next());
        }

        Ok(self.next.as_ref())
    }
}

impl<I> FallibleIterator for Peekable<I>
    where I: FallibleIterator
{
    type Item = I::Item;
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<I::Item>, I::Error> {
        if let Some(next) = self.next.take() {
            return Ok(Some(next));
        }

        self.it.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let mut hint = self.it.size_hint();
        if self.next.is_some() {
            hint.0 = hint.0.saturating_add(1);
            hint.1 = hint.1.and_then(|h| h.checked_add(1));
        }
        hint
    }
}

/// An iterator which yields elements of the underlying iterator in reverse
/// order.
#[derive(Debug)]
pub struct Rev<I>(I);

impl<I> FallibleIterator for Rev<I>
    where I: DoubleEndedFallibleIterator
{
    type Item = I::Item;
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<I::Item>, I::Error> {
        self.0.next_back()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }

    #[inline]
    fn count(self) -> Result<usize, I::Error> {
        self.0.count()
    }
}

impl<I> DoubleEndedFallibleIterator for Rev<I>
    where I: DoubleEndedFallibleIterator
{
    #[inline]
    fn next_back(&mut self) -> Result<Option<I::Item>, I::Error> {
        self.0.next()
    }
}

/// An iterator which yeilds a limited number of elements from the underlying
/// iterator.
#[derive(Debug)]
pub struct Take<I> {
    it: I,
    remaining: usize,
}

impl<I> FallibleIterator for Take<I>
    where I: FallibleIterator
{
    type Item = I::Item;
    type Error = I::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<I::Item>, I::Error> {
        if self.remaining == 0 {
            return Ok(None);
        }

        let next = self.it.next();
        if let Ok(Some(_)) = next {
            self.remaining -= 1;
        }
        next
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let hint = self.it.size_hint();
        (cmp::min(hint.0, self.remaining), hint.1.map(|n| cmp::min(n, self.remaining)))
    }
}

/// An iterator that yields pairs of this iterator's and another iterator's
/// values.
#[derive(Debug)]
pub struct Zip<T, U>(T, U);

impl<T, U> FallibleIterator for Zip<T, U>
    where T: FallibleIterator,
          U: FallibleIterator<Error = T::Error>
{
    type Item = (T::Item, U::Item);
    type Error = T::Error;

    #[inline]
    fn next(&mut self) -> Result<Option<(T::Item, U::Item)>, T::Error> {
        match (try!(self.0.next()), try!(self.1.next())) {
            (Some(a), Some(b)) => Ok(Some((a, b))),
            _ => Ok(None),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let a = self.0.size_hint();
        let b = self.1.size_hint();

        let low = cmp::min(a.0, b.0);

        let high = match (a.1, b.1) {
            (Some(a), Some(b)) => Some(cmp::min(a, b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };

        (low, high)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn _is_object_safe(_: &FallibleIterator<Item = (), Error = ()>) {}
}