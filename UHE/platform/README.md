# platform

Platform-level code only: what changes when the operating system changes.

- windowing and input (`eyes`) — window, surface handle, events, polled input
- anything that names `std::os::windows` or `std::os::linux`

## What does not belong here

**GPU backends live in `UHE/Stormbreaker/`.** Vulkan and wgpu are graphics APIs,
not platforms: the same Vulkan backend runs on Linux and Windows, and wgpu runs
on Vulkan, Metal and DX12 from one implementation. Putting them here would say
"this is the Windows graphics" and "this is the Linux graphics", which is exactly
the false choice the two-backend design exists to avoid.

The rule this directory encodes: **if moving to another operating system would
mean rewriting it, it is not platform code.**

The backend boundary is `chassis::Gpu` and `chassis::Resources` in `UHE/src/rhi`.
Neither backend imports the other, and nothing outside `UHE/Stormbreaker/`
mentions `ash` or `wgpu`.
