# Nexusq
**A multi producer multi consumer broadcast channel 
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
|         | 1w,1r  | 2w,1r  | 3w,1r  | 1w,2r  | 2w,2r  | 3w,2r  | 1w,3r  | 2w,3r  | 3w,3r  |
|---------|--------|--------|--------|--------|--------|--------|--------|--------|--------|
| nexusq  | 26.945 | 13.651 | 9.112  | 17.158 | 8.6075 | 3.822  | 10.282 | 6.5375 | 1.9437 |
| multiq  | 19.503 | 15.392 | 14.753 | 16.412 | 11.519 | 6.8163 | 10.345 | 4.9533 | 4.9182 |
| multiq2 | 13.94  | 12.128 | 7.9346 | 17.009 | 11.239 | 6.5446 | 12.122 | 5.0413 | 6.0842 |

![benchmark results](benchmark_results/bench.jpg)