//! # amethyst_assets
//!
//! Asset management crate.
//! Designed with the following goals in mind:
//!
//! * extensibility
//! * asynchronous & parallel using rayon
//! * allow different sources

#![warn(missing_docs, rust_2018_idioms, rust_2018_compatibility)]

use amethyst_core;

#[macro_use]
extern crate derivative;
#[macro_use]
extern crate error_chain;

#[macro_use]
extern crate log;

#[macro_use]
extern crate serde;
use shred;
#[macro_use]
extern crate shred_derive;

#[macro_use]
#[cfg(feature = "profiler")]
extern crate thread_profiler;

pub use crate::{
    asset::{Asset, Format, FormatValue, SimpleFormat},
    cache::Cache,
    error::{Error, ErrorKind, Result, ResultExt},
    formats::RonFormat,
    helper::AssetLoaderSystemData,
    loader::Loader,
    prefab::{AssetPrefab, Prefab, PrefabData, PrefabError, PrefabLoader, PrefabLoaderSystem},
    progress::{Completion, Progress, ProgressCounter, Tracker},
    reload::{HotReloadBundle, HotReloadStrategy, HotReloadSystem, Reload, SingleFile},
    source::{Directory, Source},
    storage::{AssetStorage, Handle, ProcessingState, Processor, WeakHandle},
};
#[cfg(feature = "json")]
pub use crate::formats::JsonFormat;

mod asset;
mod cache;
mod error;
mod formats;
mod helper;
mod loader;
mod prefab;
mod progress;
mod reload;
mod source;
mod storage;
