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

# Benchmarks
*Coming soon*