// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use core::option::*;
use core::markers;
use core::mem;
use core::clone::Clone;

#[test]
fn test_get_ptr() {
    unsafe {
        let x = box 0i;
        let addr_x: *const int = mem::transmute(&*x);
        let opt = Some(x);
        let y = opt.unwrap();
        let addr_y: *const int = mem::transmute(&*y);
        assert_eq!(addr_x, addr_y);
    }
}

#[test]
fn test_get_str() {
    let x = "test".to_string();
    let addr_x = x.as_ptr();
    let opt = Some(x);
    let y = opt.unwrap();
    let addr_y = y.as_ptr();
    assert_eq!(addr_x, addr_y);
}

#[test]
fn test_get_resource() {
    use std::rc::Rc;
    use core::cell::RefCell;

    struct R {
       i: Rc<RefCell<int>>,
    }

    #[unsafe_destructor]
    impl Drop for R {
       fn drop(&mut self) {
            let ii = &*self.i;
            let i = *ii.borrow();
            *ii.borrow_mut() = i + 1;
        }
    }

    fn r(i: Rc<RefCell<int>>) -> R {
        R {
            i: i
        }
    }

    let i = Rc::new(RefCell::new(0i));
    {
        let x = r(i.clone());
        let opt = Some(x);
        let _y = opt.unwrap();
    }
    assert_eq!(*i.borrow(), 1);
}

#[test]
fn test_option_dance() {
    let x = Some(());
    let mut y = Some(5i);
    let mut y2 = 0;
    for _x in x.iter() {
        y2 = y.take().unwrap();
    }
    assert_eq!(y2, 5);
    assert!(y.is_none());
}

#[test] #[should_fail]
fn test_option_too_much_dance() {
    let mut y = Some(markers::NoCopy);
    let _y2 = y.take().unwrap();
    let _y3 = y.take().unwrap();
}

#[test]
fn test_and() {
    let x: Option<int> = Some(1i);
    assert_eq!(x.and(Some(2i)), Some(2));
    assert_eq!(x.and(None::<int>), None);

    let x: Option<int> = None;
    assert_eq!(x.and(Some(2i)), None);
    assert_eq!(x.and(None::<int>), None);
}

#[test]
fn test_and_then() {
    let x: Option<int> = Some(1);
    assert_eq!(x.and_then(|x| Some(x + 1)), Some(2));
    assert_eq!(x.and_then(|_| None::<int>), None);

    let x: Option<int> = None;
    assert_eq!(x.and_then(|x| Some(x + 1)), None);
    assert_eq!(x.and_then(|_| None::<int>), None);
}

#[test]
fn test_or() {
    let x: Option<int> = Some(1);
    assert_eq!(x.or(Some(2)), Some(1));
    assert_eq!(x.or(None), Some(1));

    let x: Option<int> = None;
    assert_eq!(x.or(Some(2)), Some(2));
    assert_eq!(x.or(None), None);
}

#[test]
fn test_or_else() {
    let x: Option<int> = Some(1);
    assert_eq!(x.or_else(|| Some(2)), Some(1));
    assert_eq!(x.or_else(|| None), Some(1));

    let x: Option<int> = None;
    assert_eq!(x.or_else(|| Some(2)), Some(2));
    assert_eq!(x.or_else(|| None), None);
}

#[test]
fn test_unwrap() {
    assert_eq!(Some(1i).unwrap(), 1);
    let s = Some("hello".to_string()).unwrap();
    assert_eq!(s, "hello");
}

#[test]
#[should_fail]
fn test_unwrap_panic1() {
    let x: Option<int> = None;
    x.unwrap();
}

#[test]
#[should_fail]
fn test_unwrap_panic2() {
    let x: Option<String> = None;
    x.unwrap();
}

#[test]
fn test_unwrap_or() {
    let x: Option<int> = Some(1);
    assert_eq!(x.unwrap_or(2), 1);

    let x: Option<int> = None;
    assert_eq!(x.unwrap_or(2), 2);
}

#[test]
fn test_unwrap_or_else() {
    let x: Option<int> = Some(1);
    assert_eq!(x.unwrap_or_else(|| 2), 1);

    let x: Option<int> = None;
    assert_eq!(x.unwrap_or_else(|| 2), 2);
}

#[test]
fn test_iter() {
    let val = 5i;

    let x = Some(val);
    let mut it = x.iter();

    assert_eq!(it.size_hint(), (1, Some(1)));
    assert_eq!(it.next(), Some(&val));
    assert_eq!(it.size_hint(), (0, Some(0)));
    assert!(it.next().is_none());
}

#[test]
fn test_mut_iter() {
    let val = 5i;
    let new_val = 11i;

    let mut x = Some(val);
    {
        let mut it = x.iter_mut();

        assert_eq!(it.size_hint(), (1, Some(1)));

        match it.next() {
            Some(interior) => {
                assert_eq!(*interior, val);
                *interior = new_val;
            }
            None => assert!(false),
        }

        assert_eq!(it.size_hint(), (0, Some(0)));
        assert!(it.next().is_none());
    }
    assert_eq!(x, Some(new_val));
}

#[test]
fn test_ord() {
    let small = Some(1.0f64);
    let big = Some(5.0f64);
    let nan = Some(0.0f64/0.0);
    assert!(!(nan < big));
    assert!(!(nan > big));
    assert!(small < big);
    assert!(None < big);
    assert!(big > None);
}

/* FIXME(#20575)
#[test]
fn test_collect() {
    let v: Option<Vec<int>> = range(0i, 0).map(|_| Some(0i)).collect();
    assert!(v == Some(vec![]));

    let v: Option<Vec<int>> = range(0i, 3).map(|x| Some(x)).collect();
    assert!(v == Some(vec![0, 1, 2]));

    let v: Option<Vec<int>> = range(0i, 3).map(|x| {
        if x > 1 { None } else { Some(x) }
    }).collect();
    assert!(v == None);

    // test that it does not take more elements than it needs
    let mut functions: [Box<Fn() -> Option<()>>; 3] =
        [box || Some(()), box || None, box || panic!()];

    let v: Option<Vec<()>> = functions.iter_mut().map(|f| (*f)()).collect();

    assert!(v == None);
}
*/

#[test]
fn test_cloned() {
    let val1 = 1u32;
    let mut val2 = 2u32;
    let val1_ref = &val1;
    let opt_none: Option<&'static u32> = None;
    let opt_ref = Some(&val1);
    let opt_ref_ref = Some(&val1_ref);
    let opt_mut_ref = Some(&mut val2);

    // None works
    assert_eq!(opt_none.clone(), None);
    assert_eq!(opt_none.cloned(), None);

    // Mutable refs work
    assert_eq!(opt_mut_ref.cloned(), Some(2u32));

    // Immutable ref works
    assert_eq!(opt_ref.clone(), Some(&val1));
    assert_eq!(opt_ref.cloned(), Some(1u32));

    // Double Immutable ref works
    assert_eq!(opt_ref_ref.clone(), Some(&val1_ref));
    assert_eq!(opt_ref_ref.clone().cloned(), Some(&val1));
    assert_eq!(opt_ref_ref.cloned().cloned(), Some(1u32));
}
