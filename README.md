# Nexusq
**A fast multi producer multi consumer broadcast channel 
based on [LMAX Disruptor](https://github.com/LMAX-Exchange/disruptor)**

## What is it?
* Multi producer, multi consumer broadcasting channel (all consumers see all messages)
* Built on top of a fixed size ring buffer and uses *almost* no locks
  * Locks are only used when creating/removing a consumer, often these operations will not 
  block other producers or consumers and when it does it's minuscule.
* Supports batch reads and writes which also come with a nice performance boost, especially for copy types.
* Provides both a blocking and async API which can be used interchangeably on the same channel.

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