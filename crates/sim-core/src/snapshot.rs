//! Deterministic, versioned snapshot of the full world state.
//!
//! The canonical shareable unit (and, later, a multiplayer keyframe) is a **state** snapshot.
//! We serialize the *entire* `World` — including the full slot layout, every organism's brain
//! (variable-topology nodes/connections + recurrent activations), the free-list, and next-id
//! — so a reloaded world reproduces byte-identical future ticks
//! (`snapshot@K → N == uninterrupted@N`).
//!
//! Hand-rolled little-endian with a magic + version header; floats stored as raw `to_bits()`.

use crate::brain::{Brain, Conn};
use crate::organism::Organisms;
use crate::params::WorldParams;
use crate::world::World;

const MAGIC: u32 = 0x45564F34; // "EVO4"
const VERSION: u16 = 4;

#[derive(Debug, PartialEq, Eq)]
pub enum SnapshotError {
    Truncated,
    BadMagic,
    BadVersion(u16),
}

pub fn to_bytes(w: &World) -> Vec<u8> {
    let mut o = Writer { buf: Vec::new() };
    o.u32(MAGIC);
    o.u16(VERSION);
    o.u64(w.seed);
    o.u64(w.tick_count);
    write_params(&mut o, &w.params);

    o.u32(w.field.len() as u32);
    for &c in &w.field {
        o.i64(c);
    }
    o.u32(w.signal.len() as u32);
    for &s in &w.signal {
        o.i64(s);
    }
    o.u32(w.blooms.len() as u32);
    for &(bx, by) in &w.blooms {
        o.f32(bx);
        o.f32(by);
    }

    write_orgs(&mut o, &w.orgs);
    o.buf
}

pub fn from_bytes(bytes: &[u8]) -> Result<World, SnapshotError> {
    let mut r = Reader { buf: bytes, pos: 0 };
    if r.u32()? != MAGIC {
        return Err(SnapshotError::BadMagic);
    }
    let ver = r.u16()?;
    if ver != VERSION {
        return Err(SnapshotError::BadVersion(ver));
    }
    let seed = r.u64()?;
    let tick_count = r.u64()?;
    let params = read_params(&mut r)?;

    let flen = r.u32()? as usize;
    let mut field = Vec::with_capacity(flen);
    for _ in 0..flen {
        field.push(r.i64()?);
    }
    let slen = r.u32()? as usize;
    let mut signal = Vec::with_capacity(slen);
    for _ in 0..slen {
        signal.push(r.i64()?);
    }
    let blen = r.u32()? as usize;
    let mut blooms = Vec::with_capacity(blen);
    for _ in 0..blen {
        blooms.push((r.f32()?, r.f32()?));
    }

    let orgs = read_orgs(&mut r)?;
    Ok(World::from_parts(
        params, seed, tick_count, field, signal, blooms, orgs,
    ))
}

fn write_params(o: &mut Writer, p: &WorldParams) {
    o.f32(p.width);
    o.f32(p.height);
    o.u32(p.grid_w);
    o.u32(p.grid_h);
    o.i64(p.field_cap);
    o.i64(p.field_regrow);
    o.u32(p.day_period);
    o.u32(p.bloom_count);
    o.i64(p.bloom_boost);
    o.i64(p.emit_scale);
    o.i64(p.signal_cap);
    o.f32(p.accel_scale);
    o.f32(p.max_speed);
    o.i64(p.move_cost_coeff);
    o.f32(p.sense_radius);
    o.f32(p.contact_radius);
    o.f32(p.predation_size_ratio);
    o.i64(p.bite_amount);
    o.i64(p.predation_gain_num);
    o.i64(p.predation_gain_den);
    o.i64(p.basal_upkeep);
    o.i64(p.brain_cost);
    o.i64(p.size_upkeep);
    o.i64(p.eat_rate);
    o.i64(p.death_deposit);
    o.u32(p.max_age);
    o.i64(p.repro_threshold);
    o.i64(p.repro_cost);
    o.i64(p.offspring_energy);
    o.f32(p.spawn_radius);
    o.f32(p.mutation_rate);
    o.f32(p.mutation_delta);
    o.f32(p.weight_mut_delta);
    o.f32(p.add_conn_prob);
    o.f32(p.add_node_prob);
    o.u32(p.initial_population);
    o.i64(p.initial_energy);
    o.u32(p.max_population);
}

fn read_params(r: &mut Reader) -> Result<WorldParams, SnapshotError> {
    Ok(WorldParams {
        width: r.f32()?,
        height: r.f32()?,
        grid_w: r.u32()?,
        grid_h: r.u32()?,
        field_cap: r.i64()?,
        field_regrow: r.i64()?,
        day_period: r.u32()?,
        bloom_count: r.u32()?,
        bloom_boost: r.i64()?,
        emit_scale: r.i64()?,
        signal_cap: r.i64()?,
        accel_scale: r.f32()?,
        max_speed: r.f32()?,
        move_cost_coeff: r.i64()?,
        sense_radius: r.f32()?,
        contact_radius: r.f32()?,
        predation_size_ratio: r.f32()?,
        bite_amount: r.i64()?,
        predation_gain_num: r.i64()?,
        predation_gain_den: r.i64()?,
        basal_upkeep: r.i64()?,
        brain_cost: r.i64()?,
        size_upkeep: r.i64()?,
        eat_rate: r.i64()?,
        death_deposit: r.i64()?,
        max_age: r.u32()?,
        repro_threshold: r.i64()?,
        repro_cost: r.i64()?,
        offspring_energy: r.i64()?,
        spawn_radius: r.f32()?,
        mutation_rate: r.f32()?,
        mutation_delta: r.f32()?,
        weight_mut_delta: r.f32()?,
        add_conn_prob: r.f32()?,
        add_node_prob: r.f32()?,
        initial_population: r.u32()?,
        initial_energy: r.i64()?,
        max_population: r.u32()?,
    })
}

fn write_brain(o: &mut Writer, br: &Brain) {
    o.u32(br.n_hidden as u32);
    o.u32(br.conns.len() as u32);
    for c in &br.conns {
        o.u32(c.from);
        o.u32(c.to);
        o.f32(c.w);
        o.bool(c.enabled);
    }
    o.u32(br.bias.len() as u32);
    for &x in &br.bias {
        o.f32(x);
    }
    o.u32(br.act.len() as u32);
    for &x in &br.act {
        o.f32(x);
    }
}

fn read_brain(r: &mut Reader) -> Result<Brain, SnapshotError> {
    let n_hidden = r.u32()? as u16;
    let nc = r.u32()? as usize;
    let mut conns = Vec::with_capacity(nc);
    for _ in 0..nc {
        conns.push(Conn {
            from: r.u32()?,
            to: r.u32()?,
            w: r.f32()?,
            enabled: r.bool()?,
        });
    }
    let nb = r.u32()? as usize;
    let mut bias = Vec::with_capacity(nb);
    for _ in 0..nb {
        bias.push(r.f32()?);
    }
    let na = r.u32()? as usize;
    let mut act = Vec::with_capacity(na);
    for _ in 0..na {
        act.push(r.f32()?);
    }
    Ok(Brain {
        n_hidden,
        bias,
        act,
        conns,
    })
}

fn write_orgs(o: &mut Writer, s: &Organisms) {
    let cap = s.capacity();
    o.u32(cap as u32);
    for i in 0..cap {
        o.bool(s.alive[i]);
        o.u32(s.id[i]);
        o.f32(s.px[i]);
        o.f32(s.py[i]);
        o.f32(s.vx[i]);
        o.f32(s.vy[i]);
        o.i64(s.energy[i]);
        o.u32(s.age[i]);
        o.u32(s.parent[i]);
        o.u64(s.birth_tick[i]);
        o.f32(s.g_size[i]);
        o.f32(s.g_metab[i]);
        o.f32(s.g_repro[i]);
        o.u8(s.cr[i]);
        o.u8(s.cg[i]);
        o.u8(s.cb[i]);
        o.f32(s.carnivory[i]);
        write_brain(o, &s.brains[i]);
    }
    o.u32(s.free.len() as u32);
    for &f in &s.free {
        o.u32(f);
    }
    o.u32(s.next_id);
    o.u32(s.count);
}

fn read_orgs(r: &mut Reader) -> Result<Organisms, SnapshotError> {
    let cap = r.u32()? as usize;
    let mut s = Organisms::with_capacity(cap);
    for _ in 0..cap {
        s.alive.push(r.bool()?);
        s.id.push(r.u32()?);
        s.px.push(r.f32()?);
        s.py.push(r.f32()?);
        s.vx.push(r.f32()?);
        s.vy.push(r.f32()?);
        s.energy.push(r.i64()?);
        s.age.push(r.u32()?);
        s.parent.push(r.u32()?);
        s.birth_tick.push(r.u64()?);
        s.g_size.push(r.f32()?);
        s.g_metab.push(r.f32()?);
        s.g_repro.push(r.f32()?);
        s.cr.push(r.u8()?);
        s.cg.push(r.u8()?);
        s.cb.push(r.u8()?);
        s.carnivory.push(r.f32()?);
        s.brains.push(read_brain(r)?);
    }
    let free_len = r.u32()? as usize;
    s.free = Vec::with_capacity(free_len);
    for _ in 0..free_len {
        s.free.push(r.u32()?);
    }
    s.next_id = r.u32()?;
    s.count = r.u32()?;
    Ok(s)
}

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], SnapshotError> {
        if self.pos + n > self.buf.len() {
            return Err(SnapshotError::Truncated);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, SnapshotError> {
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> Result<bool, SnapshotError> {
        Ok(self.u8()? != 0)
    }
    fn u16(&mut self) -> Result<u16, SnapshotError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, SnapshotError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, SnapshotError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, SnapshotError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, SnapshotError> {
        Ok(f32::from_bits(self.u32()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::WorldParams;

    #[test]
    fn roundtrip_preserves_hash() {
        let mut w = World::new(4242, WorldParams::default());
        for _ in 0..200 {
            w.tick(&[]);
        }
        let w2 = from_bytes(&to_bytes(&w)).expect("decode");
        assert_eq!(w.state_hash(), w2.state_hash());
    }

    #[test]
    fn snapshot_then_run_equals_uninterrupted() {
        let (seed, k, n) = (99u64, 137u64, 450u64);
        let mut a = World::new(seed, WorldParams::default());
        for _ in 0..n {
            a.tick(&[]);
        }
        let mut b = World::new(seed, WorldParams::default());
        for _ in 0..k {
            b.tick(&[]);
        }
        let mut c = from_bytes(&to_bytes(&b)).expect("decode");
        for _ in k..n {
            c.tick(&[]);
        }
        assert_eq!(a.state_hash(), c.state_hash());
    }

    #[test]
    fn bad_magic_is_rejected() {
        assert_eq!(
            from_bytes(&[0, 1, 2, 3, 4, 5]).unwrap_err(),
            SnapshotError::BadMagic
        );
    }
}
