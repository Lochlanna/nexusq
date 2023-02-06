#![allow(dead_code)]
#![doc = include_str!("../README.md")]
// #![cfg(feature = "no-std")]
// #![no_std]

#[cfg(test)]
extern crate std;

extern crate alloc;

#[cfg(test)]
mod bench_test;
mod channel;
mod wait_strategy;

pub use channel::{
    channel,
    receiver::{BroadcastReceiver, ReaderError, Receiver},
    sender::{BroadcastSender, Sender, SenderError},
};
