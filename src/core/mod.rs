// Some core capabilities (mods, worlds, backups, crash analysis) are fully
// ported but not yet surfaced in the MVP GUI; keep them without dead-code noise.
#![allow(dead_code)]

//! Pyrite launcher core.
//!
//! Frontend-agnostic Minecraft launcher logic: configuration, Mojang/Modrinth
//! API access, asset/library downloading, Java provisioning, instance management,
//! launching, and crash analysis. The Slint GUI in `crate::app` is the only
//! frontend; nothing in this module depends on it.

pub mod config;
pub mod api;
pub mod downloader;
pub mod launcher;
pub mod java;
pub mod instance;
pub mod assets;
pub mod crash_analyzer;
pub mod storage;
