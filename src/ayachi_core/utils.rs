// Made by Han_feng

use core::fmt::{Arguments, Debug, Display, Formatter, Write};
use core::ops::{Deref, DerefMut};
use embassy_executor::{SpawnToken, Spawner};
use picoserve::response::Content;

// Macros
#[macro_export]
macro_rules! stmt_join {
    ($seq:stmt; $start:stmt; $($next:stmt);* $(;)?) => {
        $start
        $(
            $seq
            $next
        )*
    };
}

macro_rules! impl_compact_format {
    ($($type:ty),*) => {
        $(
            impl CompactFormat for $type {
                fn compact_format(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
                    write!(f, "{}", self)
                }
            }
        )*
    };
}

// Traits
pub trait SpawnerExt {
    fn spawn_task<S, E>(&self, task: Result<SpawnToken<S>, E>) -> Result<(), E>;
}

pub trait ContainerExt<T>: Sized {
    type AsyncMapped<U>;

    fn async_map<U>(self, f: impl AsyncFnOnce(T) -> U) -> impl Future<Output = Self::AsyncMapped<U>>;
}

pub trait CompactFormat {
    fn compact_format(&self, f: &mut Formatter<'_>) -> core::fmt::Result;
    fn into_compact_format(self) -> CompactFormatWrapper<Self> where Self: Sized {
        CompactFormatWrapper(self)
    }
}

// Structs
pub struct FixedString<const N: usize> {
    data: [u8; N],
    length: usize,
}

#[derive(Copy, Clone)]
pub struct TextBytes<const N: usize>(pub [u8; N]);

pub struct CompactFormatWrapper<T>(T);

// Impls
impl<const N: usize> FixedString<N> {
    pub const fn new() -> Self {
        FixedString { data: [0; N], length: 0 }
    }
    
    pub fn write_format(mut self, value: impl Debug) -> Self {
        if self.write_fmt(format_args!("{:?}", value)).is_err() {
            self.length = 0;
            let _ = self.write_str("truncated");
        }
        self
    }
    
    pub fn write_formats(mut self, format_args: Arguments<'_>) -> Self {
        if self.write_fmt(format_args).is_err() {
            self.length = 0;
            let _ = self.write_str("truncated");
        }
        self
    }
    
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.data[..self.length]).unwrap_or("unknown")
    }
}

impl<T, const N: usize> From<T> for FixedString<N>
    where T: Debug
{
    fn from(value: T) -> Self {
        Self::new().write_format(value)
    }
}

impl<const N: usize> Write for FixedString<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        if self.length + bytes.len() > N {
            return Err(core::fmt::Error);
        }
        
        self.data[self.length..self.length + s.len()].copy_from_slice(bytes);
        self.length += bytes.len();
        Ok(())
    }
}

impl<const N: usize> Content for FixedString<N> {
    fn content_type(&self) -> &'static str {
        "text/plain; charset=utf-8"
    }

    fn content_length(&self) -> usize {
        self.length
    }

    async fn write_content<W: picoserve::io::Write>(self, mut writer: W) -> Result<(), W::Error> {
        writer.write_fmt(format_args!("{}", self.as_str())).await
    }
}

impl<const N: usize> TryFrom<&[u8]> for TextBytes<N> {
    type Error = core::array::TryFromSliceError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        value.try_into().map(Self)
    }
}

impl<const N: usize> Deref for TextBytes<N> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const N: usize> DerefMut for TextBytes<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<const N: usize> CompactFormat for TextBytes<N> {
    fn compact_format(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        for c in self.0.iter()
            .take_while(|&&byte| byte != 0)
            .map(|byte| ((0x20..0x7f).contains(byte) && !matches!(*byte, b'|'| b'/' | b';')).then(|| *byte as char).unwrap_or('.'))
        {
            write!(f, "{}", c)?;
        }

        Ok(())
    }
}

impl<const N: usize> defmt::Format for TextBytes<N> {
    fn format(&self, fmt: defmt::Formatter) {
        self.0.format(fmt)
    }
}

impl<T> Display for CompactFormatWrapper<T>
where T: CompactFormat
{
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        self.0.compact_format(f)
    }
}

impl<T> Debug for CompactFormatWrapper<T>
where T: CompactFormat
{
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        self.0.compact_format(f)
    }
}

impl SpawnerExt for Spawner{
    fn spawn_task<S, E>(&self, task: Result<SpawnToken<S>, E>) -> Result<(), E>
    {
        task.map(|task| self.spawn(task))
    }
}

impl<T> ContainerExt<T> for Option<T>{
    type AsyncMapped<U> = Option<U>;

    async fn async_map<U>(self, f: impl AsyncFnOnce(T) -> U) -> Self::AsyncMapped<U> {
        match self {
            Some(value) => Some(f(value).await),
            None => None,
        }
    }
}

impl<T, E> ContainerExt<T> for Result<T, E>{
    type AsyncMapped<U> = Result<U, E>;

    async fn async_map<U>(self, f: impl AsyncFnOnce(T) -> U) -> Self::AsyncMapped<U>
    {
        match self {
            Ok(value) => Ok(f(value).await),
            Err(error) => Err(error),
        }
    }
}

impl_compact_format!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64, char);

impl CompactFormat for bool {
    fn compact_format(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", *self as u8)
    }
}

impl CompactFormat for [u8] {
    fn compact_format(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        for &byte in self.iter() {
            write!(f, "{:02X}", byte)?;
        }
        Ok(())
    }
}

impl<T, const N:usize> CompactFormat for [T; N]
    where T: CompactFormat
{
    fn compact_format(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        if let Some(item) = self.get(0) {
            item.compact_format(f)?;
        }

        for elem in &self[1..] {
            f.write_char(';')?;
            elem.compact_format(f)?;
        }

        Ok(())
    }
}

impl<T> CompactFormat for Option<T>
    where T: CompactFormat
{
    fn compact_format(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        self.as_ref().map(|s| s as &dyn CompactFormat).unwrap_or(&'/').compact_format(f)
    }
}
