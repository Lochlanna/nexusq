#![allow(dead_code)]

extern crate core;

mod broadcast;

pub use broadcast::{channel, receiver::Receiver, sender::Sender};
