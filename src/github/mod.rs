//! Cliente HTTP de la API de GitHub.
//!
//! Este módulo agrupa todo lo necesario para consultar la API de GitHub
//! respetando sus límites:
//!
//! - [`GithubClient`]: el cliente, con `get_json` y `get_paginated`.
//! - [`GithubConfig`]: la configuración, con valores por defecto sensatos.
//! - [`Sleeper`]: la espera inyectable (producción: [`TokioSleeper`]).
//! - [`GithubError`]: los errores tipados, que nunca transportan el token.
//!
//! Las piezas puras viven en submodules: la lectura de la cabecera `Link`
//! ([`next_link`]) y la aritmética del límite de peticiones
//! ([`rate_limit_state`], [`should_pause_before_request`]).

pub mod client;
pub mod error;
pub mod pagination;
pub mod ratelimit;

pub use client::{GithubClient, GithubConfig, Sleeper, TokioSleeper};
pub use error::GithubError;
pub use pagination::next_link;
pub use ratelimit::{RateLimitState, rate_limit_state, should_pause_before_request};
