# Nexusq
**A fast multi producer multi consumer broadcast channel 
based on [LMAX Disruptor](https://github.com/LMAX-Exchange/disruptor)**

## What is it
Nexusq is a multi producer, multi consumer channel which allows 
broadcasting of data from many producers to many consumers. 
It’s built on top of a fixed size ring buffer and provides guaranteed delivery to
all consumers. 

Nexusq supports both batch reads and writes which also result in increased performance. Especially if the type implements Copy
Nexusq provide both a blocking and async API which can be used simultaneously on the same producer/consumer.

Nexusq makes heavy use of **unsafe** but provides a (probably) safe API. 
Development of Nexusq has been a personal learning project and there are no guarantees that
it is correct.


## Why use Nexusq?

* You need multiple producers and or multiple consumers of the same data
* You need all consumers to see **all** of the messages.
* You need both async and blocking access to the same channel.

## Why not use Nexusq?

* Gemino uses unsafe and may exhibit undefined behaviour that I haven't seen or found yet.

# Benchmarks
*Coming soon*