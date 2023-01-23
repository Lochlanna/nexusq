#![allow(dead_code)]
#![doc = include_str!("../README.md")]

mod channel;

pub use channel::{
    channel,
    receiver::{BroadcastReceiver, ReaderError, Receiver},
    sender::{BroadcastSender, Sender, SenderError},
};
