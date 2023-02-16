//! Experimental shared types that should make life easier.

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::Serialize;

const POISON_POLICY_IGNORE: u8 = (PoisonPolicy::Ignore) as u8;
const POISON_POLICY_PANIC: u8 = (PoisonPolicy::Panic) as u8;

enum PoisonPolicy {
    Ignore = 0,
    Panic = 1,
}

/// Stores a read/write projection as two boxes.
pub struct ProjectorRW<A, B> {
    ro: Box<(dyn Fn(&A) -> &B + Send + Sync)>,
    rw: Box<(dyn Fn(&mut A) -> &mut B + Send + Sync)>,
}

impl<A, B> ProjectorRW<A, B> {
    pub fn new<
        RO: (Fn(&A) -> &B) + Send + Sync + 'static,
        RW: (Fn(&mut A) -> &mut B) + Send + Sync + 'static,
    >(
        ro: RO,
        rw: RW,
    ) -> Self {
        Self {
            ro: Box::new(ro),
            rw: Box::new(rw),
        }
    }
}

/// Stores a read/write projection as two boxes.
pub struct Projector<A, B> {
    ro: Box<(dyn Fn(&A) -> &B + Send + Sync)>,
}

impl<A, B> Projector<A, B> {
    pub fn new<RO: (Fn(&A) -> &B) + Send + Sync + 'static>(ro: RO) -> Self {
        Self { ro: Box::new(ro) }
    }
}

enum RawOrProjection<L, P> {
    Lock(L),
    Projection(P),
}

impl<L: Clone, P: Clone> Clone for RawOrProjection<L, P> {
    fn clone(&self) -> Self {
        use RawOrProjection::*;
        match self {
            Lock(x) => Lock(x.clone()),
            Projection(x) => Projection(x.clone()),
        }
    }
}

#[repr(transparent)]
pub struct Shared<T> {
    inner: RawOrProjection<Arc<T>, Arc<Box<dyn SharedProjection<T> + Send + Sync>>>,
}

impl<'a, T: Serialize> Serialize for Shared<T>
where
    &'a T: Serialize + 'static,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        (**self).serialize(serializer)
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

trait SharedProjection<T> {
    fn read<'a>(&'a self) -> &'a T;
}

impl<T: Send + Sync> Shared<T> {
    pub fn new(t: T) -> Self {
        Self {
            inner: RawOrProjection::Lock(Arc::new(t)),
        }
    }
}

impl<T: Send + Sync, P: Send + Sync> SharedProjection<P> for (Shared<T>, Arc<Projector<T, P>>) {
    fn read<'a>(&'a self) -> &'a P {
        (self.1.ro)(&*self.0)
    }
}

impl<T> std::ops::Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        use RawOrProjection::*;
        match &self.inner {
            Lock(x) => &*x,
            Projection(x) => x.read(),
        }
    }
}

impl<T: Send + Sync> Shared<T> {
    pub fn project_fn<P: Send + Sync + 'static, RO: (Fn(&T) -> &P) + Send + Sync + 'static>(
        &self,
        ro: RO,
    ) -> Shared<P>
    where
        T: 'static,
    {
        let projectable = (self.clone(), Arc::new(Projector::new(ro)));
        let projectable: Box<dyn SharedProjection<P> + Send + Sync> = Box::new(projectable);
        Shared {
            inner: RawOrProjection::Projection(Arc::new(projectable)),
        }
    }
}

#[repr(transparent)]
pub struct SharedRW<T: Send + Sync, const POISON_POLICY: u8 = { POISON_POLICY_PANIC }> {
    inner: RawOrProjection<Arc<RwLock<T>>, Arc<Box<dyn SharedRWProjection<T> + Send + Sync>>>,
}

trait SharedRWProjection<T> {
    fn lock_read<'a>(&'a self) -> SharedReadLock<'a, T>;
    fn lock_write<'a>(&'a self) -> SharedWriteLock<'a, T>;
}

impl<T: Send + Sync, P: Send + Sync, const POISON_POLICY: u8> SharedRWProjection<P>
    for (SharedRW<T, POISON_POLICY>, Arc<ProjectorRW<T, P>>)
{
    fn lock_read<'a>(&'a self) -> SharedReadLock<'a, P> {
        struct HiddenLock<'a, T, P> {
            lock: SharedReadLock<'a, T>,
            projector: Arc<ProjectorRW<T, P>>,
        }

        impl<'a, T, P> std::ops::Deref for HiddenLock<'a, T, P> {
            type Target = P;
            fn deref(&self) -> &Self::Target {
                (self.projector.ro)(&*self.lock)
            }
        }

        let lock = HiddenLock {
            lock: self.0.lock_read(),
            projector: self.1.clone(),
        };

        SharedReadLock {
            lock: RawOrProjection::Projection(Box::new(lock)),
        }
    }

    fn lock_write<'a>(&'a self) -> SharedWriteLock<'a, P> {
        struct HiddenLock<'a, T, P> {
            lock: SharedWriteLock<'a, T>,
            projector: Arc<ProjectorRW<T, P>>,
        }

        impl<'a, T, P> std::ops::Deref for HiddenLock<'a, T, P> {
            type Target = P;
            fn deref(&self) -> &Self::Target {
                (self.projector.ro)(&*self.lock)
            }
        }

        impl<'a, T, P> std::ops::DerefMut for HiddenLock<'a, T, P> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                (self.projector.rw)(&mut *self.lock)
            }
        }

        let lock = HiddenLock {
            lock: self.0.lock_write(),
            projector: self.1.clone(),
        };

        SharedWriteLock {
            lock: RawOrProjection::Projection(Box::new(lock)),
        }
    }
}

impl<T: Send + Sync, const POISON_POLICY: u8> Clone for SharedRW<T, POISON_POLICY> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[repr(transparent)]
pub struct SharedReadLock<'a, T> {
    lock: RawOrProjection<RwLockReadGuard<'a, T>, Box<dyn std::ops::Deref<Target = T> + 'a>>,
}

#[repr(transparent)]
pub struct SharedWriteLock<'a, T> {
    lock: RawOrProjection<RwLockWriteGuard<'a, T>, Box<dyn std::ops::DerefMut<Target = T> + 'a>>,
}

impl<'a, T> std::ops::Deref for SharedReadLock<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        use RawOrProjection::*;
        match &self.lock {
            Lock(x) => &*x,
            Projection(x) => &*x,
        }
    }
}

impl<'a, T> std::ops::Deref for SharedWriteLock<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        use RawOrProjection::*;
        match &self.lock {
            Lock(x) => &*x,
            Projection(x) => &*x,
        }
    }
}

impl<'a, T> std::ops::DerefMut for SharedWriteLock<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        use RawOrProjection::*;
        match &mut self.lock {
            Lock(x) => &mut *x,
            Projection(x) => &mut *x,
        }
    }
}

impl<T: Send + Sync> SharedRW<T> {
    pub fn new(t: T) -> SharedRW<T, POISON_POLICY_PANIC> {
        SharedRW {
            inner: RawOrProjection::Lock(Arc::new(RwLock::new(t))),
        }
    }
}

impl<T: Send + Sync, const POISON_POLICY: u8> SharedRW<T, POISON_POLICY> {
    pub fn project<P: Send + Sync + 'static, I: Into<Arc<ProjectorRW<T, P>>>>(
        &self,
        projector: I,
    ) -> SharedRW<P, POISON_POLICY>
    where
        T: 'static,
    {
        let projectable = (self.clone(), projector.into());
        let projectable: Box<dyn SharedRWProjection<P> + Send + Sync> = Box::new(projectable);
        SharedRW {
            inner: RawOrProjection::Projection(Arc::new(projectable)),
        }
    }

    pub fn project_fn<
        P: Send + Sync + 'static,
        RO: (Fn(&T) -> &P) + Send + Sync + 'static,
        RW: (Fn(&mut T) -> &mut P) + Send + Sync + 'static,
    >(
        &self,
        ro: RO,
        rw: RW,
    ) -> SharedRW<P, POISON_POLICY>
    where
        T: 'static,
    {
        let projectable = (self.clone(), Arc::new(ProjectorRW::new(ro, rw)));
        let projectable: Box<dyn SharedRWProjection<P> + Send + Sync> = Box::new(projectable);
        SharedRW {
            inner: RawOrProjection::Projection(Arc::new(projectable)),
        }
    }

    pub fn lock_read(&self) -> SharedReadLock<T> {
        match &self.inner {
            RawOrProjection::Lock(lock) => {
                let res = lock.read();
                let lock = match res {
                    Ok(lock) => lock,
                    Err(err) => err.into_inner(),
                };
                SharedReadLock {
                    lock: RawOrProjection::Lock(lock),
                }
            }
            RawOrProjection::Projection(p) => p.lock_read(),
        }
    }

    pub fn lock_write(&self) -> SharedWriteLock<T> {
        match &self.inner {
            RawOrProjection::Lock(lock) => {
                let res = lock.write();
                let lock = match res {
                    Ok(lock) => lock,
                    Err(err) => err.into_inner(),
                };
                SharedWriteLock {
                    lock: RawOrProjection::Lock(lock),
                }
            }
            RawOrProjection::Projection(p) => p.lock_write(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::{Shared, SharedRW};

    #[test]
    pub fn test_shared() {
        let shared = Shared::new(1);
        assert_eq!(*shared, 1);
    }

    #[test]
    pub fn test_shared_projection() {
        let shared = Shared::new((1, 2));
        let shared_proj = shared.project_fn(|x| &x.0);
        assert_eq!(*shared_proj, 1);
        let shared_proj = shared.project_fn(|x| &x.1);
        assert_eq!(*shared_proj, 2);
    }

    #[test]
    pub fn test_shared_rw() {
        let shared = SharedRW::new(1);
        *shared.lock_write() += 1;
        assert_eq!(*shared.lock_read(), 2);
    }

    #[test]
    pub fn test_shared_rw_projection() {
        let shared = SharedRW::new((1, 1));
        let shared_1 = shared.project_fn(|x| &x.0, |x| &mut (x.0));
        let shared_2 = shared.project_fn(|x| &x.1, |x| &mut (x.1));

        *shared_1.lock_write() += 1;
        *shared_2.lock_write() += 10;

        assert_eq!(*shared.lock_read(), (2, 11));
    }
}
