//! An **open, compositional genome** — the substrate for genuinely emergent properties.
//!
//! Instead of a fixed record of named traits, a genome is a variable-length `Vec<Gene>` of
//! small typed tokens. A deterministic [`develop`] pass compiles that gene list into a
//! [`Phenotype`] — the scalar traits `world::step_organism` already reads. So a "trait" is now
//! an **emergent sum** of many genes, not a single field, and gene **duplication** gives the
//! neutral material later divergence (and later gene kinds) build novelty from.
//!
//! This mirrors the brain's open-endedness ([`crate::brain`]): a variable-length list, mutation
//! operators (point / duplicate / delete / add), and content-hashed ids (never a global
//! counter). Determinism is preserved by accumulating trait contributions in exact `i64`
//! fixed-point (milli-units) and doing the single float compose+clamp per channel at the end.

use crate::brain::N_CHAN;
use crate::math::Scalar;
use crate::rng::{splitmix64, Pcg32};

/// Emergent scalar-trait channels a [`GeneKind::TraitMod`] can additively push. APPEND-ONLY:
/// the discriminant is serialized and hashed, so never renumber an existing channel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Channel {
    Size = 0,
    Metabolism = 1,
    Repro = 2,
    Habitat = 3,
    Diet = 4,
    ColorR = 5,
    ColorG = 6,
    ColorB = 7,
    /// Stickiness (M3): the probability a newborn stays physically bonded to its parent. High
    /// adhesion lineages leave offspring attached, so bodies (bonded cell clusters) can emerge —
    /// or not, if the crowding cost of clumping outweighs the gape-limited-predation benefit.
    /// Founders carry a small spread (standing variation for selection to act on), like every
    /// other trait; whether bonding actually spreads is left entirely to selection.
    Adhesion = 8,
}
pub const N_CHANNELS: usize = 9;

/// A gene's payload. The tag discriminant is serialized and hashed, so it is APPEND-ONLY.
///
/// The chemistry genes (`Emit`/`Uptake`/`Sense`/`Resist`) carry a **chemical channel** in
/// `0..N_CHAN` — a different namespace from the trait `Channel` above. Their meaning (is a
/// substance food, poison, or a signal?) is not assigned by us; it emerges from which genes a
/// lineage carries. Tags are APPEND-ONLY (serialized + hashed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GeneKind {
    /// Additively modifies one emergent trait `channel` by `amount` (fixed-point milli-units;
    /// raw color units for the `Color*` channels). A trait is the SUM of its `TraitMod`s.
    TraitMod { channel: u8, amount: i16 },
    /// Neutral drift material — no phenotype effect. The reservoir duplication draws from.
    Junk { payload: u32 },
    /// Excrete chemical `channel` as a metabolic byproduct, strength `rate`.
    Emit { channel: u8, rate: i16 },
    /// Perceive chemical `channel` (feeds brain input slot `IN_CHAN0 + channel`); `gain` reserved.
    Sense { channel: u8, gain: i16 },
    /// Reserved for later reactions/effects.
    React { param: u16, threshold: i16 },
    /// Absorb chemical `channel` for energy (lossily), efficiency `eff`. Makes that substance
    /// *food* for this lineage.
    Uptake { channel: u8, eff: i16 },
    /// Resistance to chemical `channel`'s toxicity — the same substance that poisons others is
    /// harmless to this lineage.
    Resist { channel: u8 },
}

/// One gene: a content-hashed identity (the homology marker / NEAT-innovation analog) plus its
/// payload. A duplicate gets a fresh derived id so the two copies can diverge and be told apart.
#[derive(Clone, Copy, Debug)]
pub struct Gene {
    pub id: u64,
    pub kind: GeneKind,
}

/// The open genome: a variable-length gene list. Canonical order = insertion order.
#[derive(Clone, Debug, Default)]
pub struct Genome {
    pub genes: Vec<Gene>,
}

/// The developed phenotype — the effective traits `step_organism` reads. Rebuilt from the
/// genome at every birth and on snapshot load; never itself serialized.
#[derive(Clone, Copy, Debug)]
pub struct Phenotype {
    pub size: Scalar,
    pub metabolism: Scalar,
    pub repro: Scalar,
    pub habitat: Scalar,
    pub diet: Scalar,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// Stickiness in `[0, 1]` (M3) — the probability a newborn stays bonded to its parent.
    pub adhesion: Scalar,
    // --- chemical roles (M2), developed from Emit/Uptake/Sense/Resist genes ---
    /// Bit `k` set ⇒ senses chemical channel `k` (its slot feeds the brain).
    pub sense_mask: u16,
    /// Bit `k` set ⇒ resistant to channel `k`'s toxicity.
    pub resist_mask: u16,
    /// Per-channel excretion strength (byproduct coefficient).
    pub emit_ch: [u8; N_CHAN],
    /// Per-channel uptake efficiency (channel `k` is food for this lineage when `> 0`).
    pub uptake_ch: [u8; N_CHAN],
}

/// Compile a genome into a phenotype. Deterministic: contributions accumulate as exact `i64`
/// milli-units in canonical gene order (integer add is associative → order-independent and
/// bit-identical across targets); float appears only in one `compose` per channel.
pub fn develop(g: &Genome) -> Phenotype {
    let mut acc = [0i64; N_CHANNELS];
    let mut sense_mask = 0u16;
    let mut resist_mask = 0u16;
    let mut emit_ch = [0u8; N_CHAN];
    let mut uptake_ch = [0u8; N_CHAN];
    for gene in &g.genes {
        match gene.kind {
            GeneKind::TraitMod { channel, amount } => {
                let c = channel as usize;
                if c < N_CHANNELS {
                    acc[c] += amount as i64;
                }
            }
            GeneKind::Emit { channel, rate } => {
                let k = channel as usize;
                if k < N_CHAN {
                    emit_ch[k] = emit_ch[k].saturating_add(clamp_u8_i16(rate));
                }
            }
            GeneKind::Uptake { channel, eff } => {
                let k = channel as usize;
                if k < N_CHAN {
                    uptake_ch[k] = uptake_ch[k].saturating_add(clamp_u8_i16(eff));
                }
            }
            GeneKind::Sense { channel, .. } => {
                let k = channel as usize;
                if k < N_CHAN {
                    sense_mask |= 1 << k;
                }
            }
            GeneKind::Resist { channel } => {
                let k = channel as usize;
                if k < N_CHAN {
                    resist_mask |= 1 << k;
                }
            }
            GeneKind::Junk { .. } | GeneKind::React { .. } => {}
        }
    }
    Phenotype {
        size: compose(1.0, acc[Channel::Size as usize], 0.4, 2.2),
        metabolism: compose(1.0, acc[Channel::Metabolism as usize], 0.3, 2.0),
        repro: compose(1.0, acc[Channel::Repro as usize], 0.5, 1.5),
        habitat: compose(0.5, acc[Channel::Habitat as usize], 0.0, 1.0),
        diet: compose(0.5, acc[Channel::Diet as usize], 0.0, 1.0),
        r: compose_u8(128, acc[Channel::ColorR as usize]),
        g: compose_u8(128, acc[Channel::ColorG as usize]),
        b: compose_u8(128, acc[Channel::ColorB as usize]),
        // Standing variation like every other trait (base 0, founders seed a small spread). We
        // seed *variation in the trait*, never bodies themselves — whether adhesion is selected up
        // (bonding pays) or down (crowding wins) is decided by the ecology, not by us.
        adhesion: compose(0.0, acc[Channel::Adhesion as usize], 0.0, 1.0),
        sense_mask,
        resist_mask,
        emit_ch,
        uptake_ch,
    }
}

/// Clamp a signed gene payload to a `u8` strength (negatives → 0).
#[inline]
fn clamp_u8_i16(v: i16) -> u8 {
    v.clamp(0, 255) as u8
}

/// `base + milli/1000`, clamped. `clamp` is comparison-only (no min/max intrinsic), matching the
/// existing deterministic core.
#[inline]
fn compose(base: f32, milli: i64, lo: f32, hi: f32) -> f32 {
    (base + milli as f32 / 1000.0).clamp(lo, hi)
}

#[inline]
fn compose_u8(base: i64, raw: i64) -> u8 {
    (base + raw).clamp(0, 255) as u8
}

/// Float→milli in fixed-point. `as i16` truncates toward zero (defined, cross-target stable).
#[inline]
fn milli(x: f32) -> i16 {
    (x * 1000.0) as i16
}

impl Genome {
    /// A founder genome: one `TraitMod` per channel, whose amounts reproduce the exact random
    /// trait ranges the shipped ecology is tuned for (so `develop()` yields the same starting
    /// distribution). Draw order is fixed; goldens regenerate once.
    pub fn founder(rng: &mut Pcg32) -> Genome {
        let mut genes = Vec::with_capacity(N_CHANNELS);
        // Same primitive draws, in the same order, as the old fixed founder.
        let size = 0.6 + rng.next_f32_unit() * 1.0;
        let metab = 0.6 + rng.next_f32_unit() * 0.8;
        let repro = 0.6 + rng.next_f32_unit() * 0.8;
        let habitat = rng.next_f32_unit();
        let diet = rng.next_f32_unit();
        let cr = rng.below(256) as i64;
        let cg = rng.below(256) as i64;
        let cb = rng.below(256) as i64;
        push_traitmod(&mut genes, rng, Channel::Size, milli(size - 1.0));
        push_traitmod(&mut genes, rng, Channel::Metabolism, milli(metab - 1.0));
        push_traitmod(&mut genes, rng, Channel::Repro, milli(repro - 1.0));
        push_traitmod(&mut genes, rng, Channel::Habitat, milli(habitat - 0.5));
        push_traitmod(&mut genes, rng, Channel::Diet, milli(diet - 0.5));
        push_traitmod(&mut genes, rng, Channel::ColorR, (cr - 128) as i16);
        push_traitmod(&mut genes, rng, Channel::ColorG, (cg - 128) as i16);
        push_traitmod(&mut genes, rng, Channel::ColorB, (cb - 128) as i16);
        let adhesion = rng.next_f32_unit() * 0.4; // standing variation in stickiness (0..0.4)
        push_traitmod(&mut genes, rng, Channel::Adhesion, milli(adhesion));
        Genome { genes }
    }

    /// Mutate the gene list — the open-genome analog of [`crate::brain::Brain::mutate`]:
    /// point-mutate payloads, duplicate (identity-preserving), delete, and add genes.
    pub fn mutate(&mut self, rng: &mut Pcg32, p: &crate::params::WorldParams) {
        // 1. point-mutate payloads (analog of weight jitter).
        for gene in self.genes.iter_mut() {
            if rng.chance(p.mutation_rate) {
                point_mutate(&mut gene.kind, rng, p.mutation_delta);
            }
        }
        // 2. DUPLICATE — the key novelty source, identity-preserving like the brain's add-node:
        //    a TraitMod splits its amount across the two copies (integer split preserves the sum
        //    exactly → neutral at birth, but two independently-mutable genes that can diverge).
        if rng.chance(p.gene_dup_prob) && !self.genes.is_empty() {
            let i = rng.below(self.genes.len() as u32) as usize;
            let mut copy = self.genes[i];
            if let (GeneKind::TraitMod { amount: a, .. }, GeneKind::TraitMod { amount: b, .. }) =
                (&mut self.genes[i].kind, &mut copy.kind)
            {
                let half = *a / 2;
                *b = half;
                *a -= half; // half + (a - half) == a, exactly
            }
            copy.id = new_gene_id(rng, &copy.kind);
            self.genes.push(copy);
        }
        // 3. DELETE (soft floor of one gene, so a lineage never collapses to empty).
        if rng.chance(p.gene_del_prob) && self.genes.len() > 1 {
            let i = rng.below(self.genes.len() as u32) as usize;
            self.genes.remove(i);
        }
        // 4. ADD a random small gene (a TraitMod on a random channel, occasionally Junk).
        if rng.chance(p.gene_add_prob) {
            let kind = random_small_gene(rng);
            let id = new_gene_id(rng, &kind);
            self.genes.push(Gene { id, kind });
        }
    }
}

fn push_traitmod(genes: &mut Vec<Gene>, rng: &mut Pcg32, channel: Channel, amount: i16) {
    let kind = GeneKind::TraitMod {
        channel: channel as u8,
        amount,
    };
    genes.push(Gene {
        id: new_gene_id(rng, &kind),
        kind,
    });
}

/// Jitter a gene's numeric payload in place (`Junk`/`Resist` have none, so are inert here).
fn point_mutate(kind: &mut GeneKind, rng: &mut Pcg32, delta: f32) {
    let d = (rng.next_f32_signed() * delta * 1000.0) as i32;
    match kind {
        GeneKind::TraitMod { amount, .. } => *amount = clamp_i16(*amount as i32 + d),
        GeneKind::Emit { rate, .. } => *rate = clamp_i16(*rate as i32 + d),
        GeneKind::Uptake { eff, .. } => *eff = clamp_i16(*eff as i32 + d),
        GeneKind::Sense { gain, .. } => *gain = clamp_i16(*gain as i32 + d),
        GeneKind::React { threshold, .. } => *threshold = clamp_i16(*threshold as i32 + d),
        GeneKind::Junk { .. } | GeneKind::Resist { .. } => {}
    }
}

/// A random new gene: mostly a small trait tweak, sometimes a chemistry gene on a random
/// chemical channel (so emit/uptake/toxin-resistance/perception can appear and be selected).
fn random_small_gene(rng: &mut Pcg32) -> GeneKind {
    let r = rng.below(100);
    match r {
        0..=54 => GeneKind::TraitMod {
            channel: rng.below(N_CHANNELS as u32) as u8,
            amount: (rng.next_f32_signed() * 200.0) as i16,
        },
        55..=64 => GeneKind::Junk {
            payload: rng.next_u32(),
        },
        65..=74 => GeneKind::Emit {
            channel: rng.below(N_CHAN as u32) as u8,
            rate: (rng.next_f32_unit() * 180.0) as i16,
        },
        75..=84 => GeneKind::Uptake {
            channel: rng.below(N_CHAN as u32) as u8,
            eff: (rng.next_f32_unit() * 180.0) as i16,
        },
        85..=92 => GeneKind::Sense {
            channel: rng.below(N_CHAN as u32) as u8,
            gain: 100,
        },
        _ => GeneKind::Resist {
            channel: rng.below(N_CHAN as u32) as u8,
        },
    }
}

/// A content-hashed gene id — payload bits mixed with a fresh draw from the (birth-keyed)
/// mutation stream. Reproducible, distinct, and counter-free (no globals).
fn new_gene_id(rng: &mut Pcg32, kind: &GeneKind) -> u64 {
    let salt = ((rng.next_u32() as u64) << 32) | rng.next_u32() as u64;
    splitmix64(salt ^ splitmix64(kind_hash(kind)))
}

/// A stable content hash of a gene's kind (tag + payload) — used to seed its id.
pub fn kind_hash(kind: &GeneKind) -> u64 {
    match *kind {
        GeneKind::TraitMod { channel, amount } => {
            0x01 ^ ((channel as u64) << 8) ^ ((amount as u16 as u64) << 16)
        }
        GeneKind::Junk { payload } => 0x02 ^ ((payload as u64) << 8),
        GeneKind::Emit { channel, rate } => {
            0x03 ^ ((channel as u64) << 8) ^ ((rate as u16 as u64) << 16)
        }
        GeneKind::Sense { channel, gain } => {
            0x04 ^ ((channel as u64) << 8) ^ ((gain as u16 as u64) << 16)
        }
        GeneKind::React { param, threshold } => {
            0x05 ^ ((param as u64) << 8) ^ ((threshold as u16 as u64) << 24)
        }
        GeneKind::Uptake { channel, eff } => {
            0x06 ^ ((channel as u64) << 8) ^ ((eff as u16 as u64) << 16)
        }
        GeneKind::Resist { channel } => 0x07 ^ ((channel as u64) << 8),
    }
}

#[inline]
fn clamp_i16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::WorldParams;

    fn tm(channel: Channel, amount: i16) -> Gene {
        Gene {
            id: 0,
            kind: GeneKind::TraitMod {
                channel: channel as u8,
                amount,
            },
        }
    }

    #[test]
    fn develop_is_order_independent() {
        let a = Genome {
            genes: vec![
                tm(Channel::Size, 300),
                tm(Channel::Size, -100),
                tm(Channel::Diet, 200),
            ],
        };
        let b = Genome {
            genes: vec![
                tm(Channel::Diet, 200),
                tm(Channel::Size, -100),
                tm(Channel::Size, 300),
            ],
        };
        let pa = develop(&a);
        let pb = develop(&b);
        // Sum of Size mods = 200 milli → size = 1.2; order must not matter.
        assert_eq!(pa.size.to_bits(), pb.size.to_bits());
        assert_eq!(pa.diet.to_bits(), pb.diet.to_bits());
        assert!((pa.size - 1.2).abs() < 1e-6);
    }

    #[test]
    fn duplicate_preserves_phenotype() {
        let g = Genome {
            genes: vec![tm(Channel::Size, 301)],
        };
        let before = develop(&g).size;
        // Simulate the duplicate split: 301 -> 150 + 151.
        let split = Genome {
            genes: vec![tm(Channel::Size, 150), tm(Channel::Size, 151)],
        };
        assert_eq!(develop(&split).size.to_bits(), before.to_bits());
    }

    #[test]
    fn founder_lands_in_shipped_ranges() {
        let mut rng = Pcg32::new(12345, 1);
        for _ in 0..500 {
            let ph = develop(&Genome::founder(&mut rng));
            assert!((0.4..=2.2).contains(&ph.size));
            assert!((0.3..=2.0).contains(&ph.metabolism));
            assert!((0.5..=1.5).contains(&ph.repro));
            assert!((0.0..=1.0).contains(&ph.habitat));
            assert!((0.0..=1.0).contains(&ph.diet));
        }
    }

    #[test]
    fn mutate_grows_and_stays_deterministic() {
        let p = WorldParams::default();
        let mut a = Genome::founder(&mut Pcg32::new(7, 1));
        let mut b = a.clone();
        // Same key => identical mutation sequence.
        for t in 0..200u64 {
            a.mutate(&mut Pcg32::from_key(1, 2, t), &p);
            b.mutate(&mut Pcg32::from_key(1, 2, t), &p);
        }
        assert_eq!(a.genes.len(), b.genes.len());
        assert_eq!(develop(&a).size.to_bits(), develop(&b).size.to_bits());
    }

    #[test]
    fn chemistry_genes_develop_into_roles() {
        let g = Genome {
            genes: vec![
                tm(Channel::Size, 500),
                Gene {
                    id: 1,
                    kind: GeneKind::Junk { payload: 42 },
                },
                Gene {
                    id: 2,
                    kind: GeneKind::Emit {
                        channel: 0,
                        rate: 200,
                    },
                },
                Gene {
                    id: 3,
                    kind: GeneKind::Sense {
                        channel: 1,
                        gain: 100,
                    },
                },
                Gene {
                    id: 4,
                    kind: GeneKind::Uptake {
                        channel: 0,
                        eff: 150,
                    },
                },
                Gene {
                    id: 5,
                    kind: GeneKind::Resist { channel: 2 },
                },
            ],
        };
        let ph = develop(&g);
        // Traits unaffected by chemistry genes: Size TraitMod 500 milli → 1.5.
        assert!((ph.size - 1.5).abs() < 1e-6);
        // Chemistry genes develop into roles.
        assert_eq!(ph.emit_ch[0], 200);
        assert_eq!(ph.uptake_ch[0], 150);
        assert_eq!(ph.sense_mask, 1 << 1);
        assert_eq!(ph.resist_mask, 1 << 2);
    }

    #[test]
    fn adhesion_founder_spread_and_gene_mapping() {
        // Founders carry standing variation in [0, 0.4] — not seeded bodies, just a trait spread.
        let mut rng = Pcg32::new(999, 1);
        for _ in 0..500 {
            let a = develop(&Genome::founder(&mut rng)).adhesion;
            assert!(
                (0.0..=0.4).contains(&a),
                "founder adhesion out of range: {a}"
            );
        }
        // A positive Adhesion TraitMod raises it; it clamps into [0, 1].
        let g = Genome {
            genes: vec![tm(Channel::Adhesion, 600)],
        };
        assert!((develop(&g).adhesion - 0.6).abs() < 1e-6);
        let g2 = Genome {
            genes: vec![tm(Channel::Adhesion, 4000)],
        };
        assert_eq!(develop(&g2).adhesion, 1.0);
        // Negative sums clamp to 0 (no "anti-stick").
        let g3 = Genome {
            genes: vec![tm(Channel::Adhesion, -500)],
        };
        assert_eq!(develop(&g3).adhesion, 0.0);
    }
}
