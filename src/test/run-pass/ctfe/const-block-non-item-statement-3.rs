// run-pass
#![allow(dead_code)]

type Array = [u32; {  let x = 2; 5 }];

pub fn main() {}
