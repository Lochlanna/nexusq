#![allow(dead_code)]

extern crate core;

mod channel;

pub use channel::{channel, receiver::BroadcastReceiver, sender::BroadcastSender};
