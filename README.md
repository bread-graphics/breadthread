# breadthread

[![Build Status](https://dev.azure.com/jtnunley01/gui-tools/_apis/build/status/notgull.breadthread?branchName=master)](https://dev.azure.com/jtnunley01/gui-tools/_build/latest?definitionId=17&branchName=master) 

The APIs of most system-specific GUI frameworks operate on the following principles:

* Primitives are represented by pointers to system objects that are not thread safe.
* The framework requires running an event loop on this thread.

`breadthread` aims to create a way to not only abstract over this kind of framework, but also allow it to be
thread safe.

`breadthread` provides the `BreadThread` object, which is the thread where the bread is acquired. An object 
implementing the `Controller` trait is used to build the `BreadThread` object, which dictates how it operates.
`BreadThread` proper is not `!Send`; however, it can be used to create `ThreadHandle` objects that are.

The `Controller` object defines a `Directive` type that is used to tell the `BreadThread` what to do, as well as
how running the event handler should work. The `BreadThread` also creates a "directive thread" dedicated to
receiving directives from other threads.

## License

MIT/Apache2 License
