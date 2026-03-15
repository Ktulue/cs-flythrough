pub mod bsp;
pub mod camera;
pub mod config;
pub mod input;
pub mod maplist;
// renderer is NOT exported — it owns an event loop and panics in headless environments
