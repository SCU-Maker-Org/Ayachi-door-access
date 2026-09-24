// Made by Han_feng

use core::fmt::{Arguments, Debug, Write};
use embassy_executor::{SpawnToken, Spawner};
use picoserve::response::Content;

// Macros
#[macro_export]
macro_rules! formats_args {
    ($format: literal, $($args:expr),*) => {
        [ $(format_args!($format, $args), )* ]
    };
}

// Traits
pub trait SpawnerExt {
    fn spawn_task<S, E>(&self, task: Result<SpawnToken<S>, E>) -> Result<(), E>;
}

pub trait ContainerExt<T>: Sized {
    type Mapped<U>;

    fn async_map<U, F>(self, f: F) -> impl Future<Output = Self::Mapped<U>>
    where
        F: AsyncFnOnce(T) -> U;
}

// Struct
pub struct FixedString<const N: usize> {
    data: [u8; N],
    length: usize,
}

// Impls
impl<const N: usize> FixedString<N> {
    pub const fn new() -> Self {
        FixedString { data: [0; N], length: 0 }
    }
    
    pub fn write_format(mut self, value: impl Debug) -> Self {
        let _ = self.write_fmt(format_args!("{:?}", value));
        self
    }
    
    pub fn write_formats(mut self, format_args: Arguments<'_>) -> Self {
        let _ = self.write_fmt(format_args);
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

impl SpawnerExt for Spawner{
    fn spawn_task<S, E>(&self, task: Result<SpawnToken<S>, E>) -> Result<(), E>
    {
        task.map(|task| self.spawn(task))
    }
}

impl<T> ContainerExt<T> for Option<T>{
    type Mapped<U> = Option<U>;

    async fn async_map<U, F>(self, f: F) -> Self::Mapped<U>
    where
        F: AsyncFnOnce(T) -> U
    {
        match self {
            Some(value) => Some(f(value).await),
            None => None,
        }
    }
}

impl<T, E> ContainerExt<T> for Result<T, E>{
    type Mapped<U> = Result<U, E>;

    async fn async_map<U, F>(self, f: F) -> Self::Mapped<U>
    where
        F: AsyncFnOnce(T) -> U
    {
        match self {
            Ok(value) => Ok(f(value).await),
            Err(error) => Err(error),
        }
    }
}