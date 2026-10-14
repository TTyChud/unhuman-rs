//! Adapter selection.
//!
//! ## Status
//!
//! No code. `WgpuDevice::headless` asks for an adapter and there is exactly one
//! code path, so an adapter that is not the one you got cannot happen -- yet.
//! /// The moment a device is created for a *specific* GPU (a laptop with two, a
//! headless run that must not touch the integrated one) this becomes real
//! selection, and the code will want to exist rather than be added.
