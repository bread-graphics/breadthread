# Deprecated

It was probably a bad idea to use this crate in the first place.

# breadthread

[![Build Status](https://dev.azure.com/jtnunley01/gui-tools/_apis/build/status/notgull.breadthread?branchName=master)](https://dev.azure.com/jtnunley01/gui-tools/_build/latest?definitionId=17&branchName=master) [![crates.io](https://img.shields.io/crates/v/breadthread)](https://crates.io/crates/breadthread) [![docs.rs](https://docs.rs/breadthread/badge.svg)](https://docs.rs/breadthread) 

`breadthread` provides a thread for getting that bread.

Certain APIs are thread unsafe, and it would be nice to be able to use them in a thread-safe
context, since certain runtimes require data to be `Send`. `breadthread` provides
a mechanism to export this work to another thread.

## License

MIT/Apache2 License
