//! Runtime for rosfix's downloadable assist models (the nano / mini / base tiers).
//!
//! One engine, three shapes: every tier is the same int8 transformer encoder with a
//! different [`Config`]. Models only score inputs that rosfix builds from a finding and its
//! closed candidate set; they never generate text. CPU only, single thread, no
//! dependencies, and bit-identical results on every platform (see [`net`]).

pub mod format;
pub mod net;
pub mod sha256;
pub mod tokenizer;
pub mod train;

pub use net::{Config, Model};
pub use tokenizer::Tokenizer;

/// The downloadable tiers. `builtin` (the 289-weight MLP) lives in rosfix itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Nano,
    Mini,
    Base,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Nano, Tier::Mini, Tier::Base];

    pub fn name(self) -> &'static str {
        match self {
            Tier::Nano => "nano",
            Tier::Mini => "mini",
            Tier::Base => "base",
        }
    }

    pub fn parse(s: &str) -> Option<Tier> {
        Tier::ALL.into_iter().find(|t| t.name() == s)
    }

    /// The fixed shape of each tier (8,192-token byte-level BPE vocabulary).
    pub fn config(self) -> Config {
        let (d, layers, heads, ffn, ctx) = match self {
            Tier::Nano => (128, 2, 2, 512, 128),
            Tier::Mini => (256, 4, 4, 1024, 256),
            Tier::Base => (512, 6, 8, 2048, 384),
        };
        Config {
            vocab: 8192,
            d,
            layers,
            heads,
            ffn,
            ctx,
        }
    }
}
