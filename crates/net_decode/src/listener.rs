// SPDX-FileCopyrightText: 2023 Jade Lovelace
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    any::{Any, TypeId},
    collections::BTreeMap,
    fmt,
};

use dyn_clone::DynClone;

use crate::chomp::IPTarget;

/// Simple type indexed map
#[derive(Default, Debug, Clone)]
pub struct TypeMap<V>(BTreeMap<TypeId, V>);

impl<V> TypeMap<V> {
    pub fn get<T: Any>(&self) -> Option<&V> {
        self.0.get(&TypeId::of::<T>())
    }

    pub fn insert<T: Any>(&mut self, val: V) {
        self.0.insert(TypeId::of::<T>(), val);
    }
}

/// Nanoseconds since the Unix epoch
pub type Nanos = u64;

#[derive(Clone, Debug, Default)]
pub struct TimingInfo {
    pub received_on_wire: Nanos,
    // Debating whether to put this in individual modules or have a central
    // registry. I would like to not have a central registry to make this code
    // more reusable.
    pub other_times: TypeMap<Nanos>,
}

pub trait SideData: fmt::Debug + DynClone + Send + Sync {
    /// Note massive footgun: if you are using this on Box you need to re-deref
    /// it: `(&*some_box).as_any()`. If you do not, it will wind up using the
    /// type ID of the box itself rather than the contents.
    fn as_any(&self) -> &dyn Any;
}

impl<T: fmt::Debug + Any + Sized + Send + Sync + DynClone> SideData for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

dyn_clone::clone_trait_object!(SideData);

#[derive(Clone, Debug)]
pub struct MessageMeta {
    pub timing: TimingInfo,
    pub target: IPTarget,
    pub to_client: bool,
}

/// Type which receives some kind of messages from a layer up the stack.
///
/// The `target` is the same in both directions so a flow can be tracked.
///
/// Conceptually a Listener is an actor which receives messages and side data
/// and generates zero or more messages and side data as a result. In order to
/// preserve bounded memory usage, we implement this as procedure calls rather
/// than on_data generating a Vec of events, for example.
//
// FIXME: This type seems ripe for refactoring, since owning the next Listener
// in a chain and calling forward seems to be generic behaviour.
pub trait Listener<MessageType>: Send + Sync {
    fn on_data(&mut self, timing: TimingInfo, target: IPTarget, to_client: bool, data: MessageType);

    /// Usually the expected implementation here is to forward the data along
    /// to the next `Listener` in the chain, if any.
    ///
    /// This is expected to be used to implement things that may or may not be
    /// attached to a particular connection, such as sending TLS keys into the
    /// chain.
    fn on_side_data(&mut self, data: Box<dyn SideData>);
}

#[derive(Debug, Default)]
pub struct NoOpListener {}

impl<T> Listener<T> for NoOpListener {
    fn on_data(&mut self, _timing: TimingInfo, _target: IPTarget, _to_client: bool, _data: T) {
        // do nothing! :D
    }

    fn on_side_data(&mut self, _data: Box<dyn SideData>) {}
}

#[derive(Debug, Default)]
pub struct HexDumpListener {}

impl Listener<Vec<u8>> for HexDumpListener {
    fn on_data(&mut self, _timing: TimingInfo, target: IPTarget, to_client: bool, data: Vec<u8>) {
        tracing::info!(
            "data {target:?} to_client={to_client}:\n{}",
            hexdump::HexDumper::new(&data)
        );
    }

    fn on_side_data(&mut self, _data: Box<dyn SideData>) {}
}

#[derive(Debug, Default)]
pub struct DebugListener {}

impl<T: fmt::Debug> Listener<T> for DebugListener {
    fn on_data(&mut self, _timing: TimingInfo, target: IPTarget, to_client: bool, data: T) {
        tracing::info!("data {target:?} to_client={to_client}: {data:?}");
    }

    fn on_side_data(&mut self, data: Box<dyn SideData>) {
        tracing::info!("side data: {data:?}");
    }
}
