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

pub use channel::{
    busy_channel, channel, channel_with,
    receiver::{BroadcastReceiver, Receiver, ReceiverError},
    sender::{BroadcastSender, Sender, SenderError},
    wait_strategy::*,
    ChannelHandles, Core,
};
