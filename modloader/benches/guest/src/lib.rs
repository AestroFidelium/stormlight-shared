//! Bench guest: some content, plus one of each runtime entry point.
//!
//! The content is what every runtime call pays to rebuild (guests are stateless,
//! so each entry re-runs this builder). The handlers each return one effect, so
//! the decode on the way back is exercised too.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use stormlight_mod_sdk::abi::abilities::{AbilityDescriptor, CastSpec, Params, Targeting};
use stormlight_mod_sdk::abi::common::ImpactTarget;
use stormlight_mod_sdk::abi::conditions::Condition;
use stormlight_mod_sdk::abi::ids::{AbilityId, DamageTypeId, ParamId};
use stormlight_mod_sdk::abi::impacts::{DamageFlags, Impact};
use stormlight_mod_sdk::abi::math::Value;
use stormlight_mod_sdk::context::ModContext;
use stormlight_mod_sdk::register_mod;
use stormlight_mod_sdk::runtime::GuestEffects;

#[cfg(not(feature = "large"))]
const ABILITIES: usize = 2;
#[cfg(feature = "large")]
const ABILITIES: usize = 64;

fn hit(amount: f32, dtype: DamageTypeId) -> Impact {
    Impact::Damage {
        amount: Value::Const(amount),
        dtype,
        target: ImpactTarget::ResolvedTarget,
        flags: DamageFlags::default(),
    }
}

fn ability(cooldown: ParamId, dtype: DamageTypeId) -> AbilityDescriptor {
    AbilityDescriptor {
        id: AbilityId(0),
        params: Params(vec![(cooldown, Value::Const(4.0))]),
        targeting: Targeting::Vector,
        cast: CastSpec::Instant,
        cost: Vec::new(),
        cast_gate: Condition::Always,
        on_cast_start: Vec::new(),
        on_cast: vec![hit(10.0, dtype)],
        tags: Vec::new(),
    }
}

register_mod!(|ctx: &mut ModContext| {
    let cooldown = ctx.param("cooldown");
    let dtype = ctx.damage_type("bench");
    let ping = ctx.event("ping");
    for i in 0..ABILITIES {
        ctx.ability(&format!("a{i}"), ability(cooldown, dtype));
    }
    ctx.on_tick(move |t| GuestEffects::new(vec![hit(t.tick as f32, dtype)]));
    ctx.on_trigger(ping, move |_| GuestEffects::new(vec![hit(1.0, dtype)]));
    ctx.on_handler("echo", move |call| GuestEffects::new(vec![hit(call.params.len() as f32, dtype)]));
});
