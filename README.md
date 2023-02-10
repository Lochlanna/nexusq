# Nexusq
**A multi producer multi consumer broadcast channel 
based on [LMAX Disruptor](https://github.com/LMAX-Exchange/disruptor)**

## What is it?
* Multi producer, multi consumer broadcasting channel (all consumers see all messages)
* Built on top of a fixed size ring buffer and uses *almost* no locks
  * Locks are only used when using the blocking wait strategy.

Nexusq makes use of **unsafe** but provides a (probably) safe API. 
Development of Nexusq has been a personal learning project and there are no guarantees that
it is correct.

# Benchmarks

|           | `nexus`                   | `multiq2`                       |
|:----------|:--------------------------|:------------------------------- |
| **`1,1`** | `724.74 us` (✅ **1.00x**) | `1.29 ms` (❌ *1.77x slower*)    |
| **`1,2`** | `1.05 ms` (✅ **1.00x**)   | `1.21 ms` (❌ *1.15x slower*)    |
| **`2,1`** | `1.84 ms` (✅ **1.00x**)   | `3.20 ms` (❌ *1.74x slower*)    |
| **`2,2`** | `3.34 ms` (✅ **1.00x**)   | `3.47 ms` (✅ **1.04x slower**)  |

---
Made with [criterion-table](https://github.com/nu11ptr/criterion-table)