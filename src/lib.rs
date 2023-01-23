#![allow(dead_code)]

extern crate core;

mod channel;

pub use channel::{
    channel,
    receiver::{BroadcastReceiver, ReaderError, Receiver},
    sender::{BroadcastSender, Sender, SenderError},
};
