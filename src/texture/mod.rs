/*!
A texture is an image loaded in video memory and that can be sampled in your shaders.

Textures come in ten different dimensions:

 - Textures with one dimension.
 - Textures with two dimensions.
 - Textures with two dimensions and multisampling enabled.
 - Textures with three dimensions.
 - Cube textures, which are arrays of six two-dimensional textures
   corresponding to the six faces of a cube.
 - Arrays of one-dimensional textures.
 - Arrays of two-dimensional textures.
 - Arrays of two-dimensional textures with multisampling enabled.
 - Arrays of cube textures.
 - Buffer textures, which are one-dimensional textures that are mapped to a buffer.

In addition to this, there are six kinds of texture formats:

 - The texture contains floating-point data,
   with either the `Compressed` prefix or no prefix at all.
 - The texture contains signed integers, with the `Integral` prefix.
 - The texture contains unsigned integers, with the `Unsigned` prefix.
 - The texture contains depth informations, with the `Depth` prefix.
 - The texture contains stencil informations, with the `Stencil` prefix.
 - The texture contains depth and stencil informations, with the `DepthStencil` prefix.

Each combinaison of dimensions and format corresponds to a sampler type in GLSL. For example
a `IntegralTexture3d` can only be binded to a `isampler3D` uniform in GLSL. Some combinaisons
don't exist, like `DepthBufferTexture`.

The difference between compressed textures and non-compressed textures is that you can't do
render-to-texture on the former.

The most common types of textures are `CompressedTexture2d` and `Texture2d` (the two dimensions
being the width and height), it is what you will use most of the time.

*/
use {gl, framebuffer};

#[cfg(feature = "image")]
use image;

use std::sync::Arc;
use std::rc::Rc;

use buffer::{mod, Buffer};
use uniforms::{UniformValue, IntoUniformValue, Sampler};
use {Surface, GlObject, ToGlEnum};

use self::tex_impl::TextureImplementation;

pub use self::format::{ClientFormat, TextureFormat};
pub use self::format::{UncompressedFloatFormat, UncompressedIntFormat, UncompressedUintFormat};
pub use self::format::{CompressedFormat, DepthFormat, DepthStencilFormat, StencilFormat};
pub use self::pixel::PixelValue;

mod format;
mod pixel;
mod tex_impl;

include!(concat!(env!("OUT_DIR"), "/textures.rs"));

/// Trait that describes a texture.
pub trait Texture {
	/// Returns the width in pixels of the texture.
	fn get_width(&self) -> u32;

	/// Returns the height in pixels of the texture, or `None` for one dimension textures.
	fn get_height(&self) -> Option<u32>;

	/// Returns the depth in pixels of the texture, or `None` for one or two dimension textures.
	fn get_depth(&self) -> Option<u32>;

	/// Returns the number of textures in the array, or `None` for non-arrays.
	fn get_array_size(&self) -> Option<u32>;
}

/// Trait that describes data for a one-dimensional texture.
pub trait Texture1dData {
	type Data: Send + Copy;

	/// Returns the format of the pixels.
	fn get_format(Option<Self>) -> ClientFormat;

	/// Returns a vec where each element is a pixel of the texture.
	fn into_vec(self) -> Vec< <Self as Texture1dData>::Data>;

	/// Builds a new object from raw data.
	fn from_vec(Vec< <Self as Texture1dData>::Data>) -> Self;
}

impl<P: PixelValue> Texture1dData for Vec<P> {
	type Data = P;

	fn get_format(_: Option<Vec<P>>) -> ClientFormat {
		PixelValue::get_format(None::<P>)
	}

	fn into_vec(self) -> Vec<P> {
		self
	}

	fn from_vec(data: Vec<P>) -> Vec<P> {
		data
	}
}

impl<'a, P: PixelValue + Clone> Texture1dData for &'a [P] {
	type Data = P;

	fn get_format(_: Option<&'a [P]>) -> ClientFormat {
		PixelValue::get_format(None::<P>)
	}

	fn into_vec(self) -> Vec<P> {
		self.to_vec()
	}

	fn from_vec(_: Vec<P>) -> &'a [P] {
		panic!()        // TODO: what to do here?
	}
}

/// Trait that describes data for a two-dimensional texture.
pub trait Texture2dData {
	type Data: Send + Copy;

	/// Returns the format of the pixels.
	fn get_format(Option<Self>) -> ClientFormat;

	/// Returns the dimensions of the texture.
	fn get_dimensions(&self) -> (u32, u32);

	/// Returns a vec where each element is a pixel of the texture.
	fn into_vec(self) -> Vec< <Self as Texture2dData>::Data>;

	/// Builds a new object from raw data.
	fn from_vec(Vec< <Self as Texture2dData>::Data>, width: u32) -> Self;
}

impl<P: PixelValue + Clone> Texture2dData for Vec<Vec<P>> {      // TODO: remove Clone
	type Data = P;

	fn get_format(_: Option<Vec<Vec<P>>>) -> ClientFormat {
		PixelValue::get_format(None::<P>)
	}

	fn get_dimensions(&self) -> (u32, u32) {
		(self.iter().next().map(|e| e.len()).unwrap_or(0) as u32, self.len() as u32)
	}

	fn into_vec(self) -> Vec<P> {
		self.into_iter().flat_map(|e| e.into_iter()).collect()
	}

	fn from_vec(data: Vec<P>, width: u32) -> Vec<Vec<P>> {
		data.as_slice().chunks(width as uint).map(|e| e.to_vec()).collect()
	}
}

#[cfg(feature = "image")]
impl<T, P> Texture2dData for image::ImageBuffer<Vec<T>, T, P> where T: image::Primitive + Send,
	P: PixelValue + image::Pixel<T> + Clone + Copy
{
	type Data = T;

	fn get_format(_: Option<image::ImageBuffer<Vec<T>, T, P>>) -> ClientFormat {
		PixelValue::get_format(None::<P>)
	}

	fn get_dimensions(&self) -> (u32, u32) {
		use image::GenericImage;
		self.dimensions()
	}

	fn into_vec(self) -> Vec<T> {
		use image::GenericImage;
		let (width, _) = self.dimensions();

		let raw_data = self.into_vec();

		// the image library gives use rows from bottom to top, so we need to flip them
		raw_data
			.as_slice()
			.chunks(width as uint * image::Pixel::channel_count(None::<&P>) as uint)
			.rev()
			.flat_map(|row| row.iter())
			.map(|p| p.clone())
			.collect()
	}

	fn from_vec(data: Vec<T>, width: u32) -> image::ImageBuffer<Vec<T>, T, P> {
		let pixels_size = image::Pixel::channel_count(None::<&P>);
		let height = data.len() as u32 / (width * pixels_size as u32);

		// opengl gives use rows from bottom to top, so we need to flip them
		let data = data
			.as_slice()
			.chunks(width as uint * image::Pixel::channel_count(None::<&P>) as uint)
			.rev()
			.flat_map(|row| row.iter())
			.map(|p| p.clone())
			.collect();

		image::ImageBuffer::from_raw(width, height, data).unwrap()
	}
}

#[cfg(feature = "image")]
impl Texture2dData for image::DynamicImage {
	type Data = u8;

	fn get_format(_: Option<image::DynamicImage>) -> ClientFormat {
		ClientFormat::U8U8U8U8
	}

	fn get_dimensions(&self) -> (u32, u32) {
		use image::GenericImage;
		self.dimensions()
	}

	fn into_vec(self) -> Vec<u8> {
		Texture2dData::into_vec(self.to_rgba())
	}

	fn from_vec(data: Vec<u8>, width: u32) -> image::DynamicImage {
		image::DynamicImage::ImageRgba8(Texture2dData::from_vec(data, width))
	}
}

/// Trait that describes data for a three-dimensional texture.
pub trait Texture3dData {
	type Data: Send + Copy;

	/// Returns the format of the pixels.
	fn get_format(Option<Self>) -> ClientFormat;

	/// Returns the dimensions of the texture.
	fn get_dimensions(&self) -> (u32, u32, u32);

	/// Returns a vec where each element is a pixel of the texture.
	fn into_vec(self) -> Vec< <Self as Texture3dData>::Data>;

	/// Builds a new object from raw data.
	fn from_vec(Vec< <Self as Texture3dData>::Data>, width: u32, height: u32) -> Self;
}

impl<P: PixelValue> Texture3dData for Vec<Vec<Vec<P>>> {
	type Data = P;

	fn get_format(_: Option<Vec<Vec<Vec<P>>>>) -> ClientFormat {
		PixelValue::get_format(None::<P>)
	}

	fn get_dimensions(&self) -> (u32, u32, u32) {
		(self.iter().next().and_then(|e| e.iter().next()).map(|e| e.len()).unwrap_or(0) as u32,
			self.iter().next().map(|e| e.len()).unwrap_or(0) as u32, self.len() as u32)
	}

	fn into_vec(self) -> Vec<P> {
		self.into_iter().flat_map(|e| e.into_iter()).flat_map(|e| e.into_iter()).collect()
	}

	fn from_vec(data: Vec<P>, width: u32, height: u32) -> Vec<Vec<Vec<P>>> {
		unimplemented!()        // TODO:
	}
}

/// Buffer that stores the content of a texture.
///
/// The generic type represents the type of pixels that the buffer contains.
///
/// **Note**: pixel buffers are unusable for the moment (they are not yet implemented).
pub struct PixelBuffer<T> {
	buffer: Buffer,
}

impl<T> PixelBuffer<T> where T: PixelValue {
	/// Builds a new buffer with an uninitialized content.
	pub fn new_empty(display: &super::Display, capacity: uint) -> PixelBuffer<T> {
		PixelBuffer {
			buffer: Buffer::new_empty::<buffer::PixelUnpackBuffer>(display, 1, capacity,
																   gl::DYNAMIC_READ),
		}
	}

	/// Turns a `PixelBuffer<T>` into a `PixelBuffer<U>` without any check.
	pub unsafe fn transmute<U>(self) -> PixelBuffer<U> where U: PixelValue {
		PixelBuffer { buffer: self.buffer }
	}
}


/// Struct that allows you to draw on a texture.
///
/// To obtain such an object, call `texture.as_surface()`.
pub struct TextureSurface<'a>(framebuffer::SimpleFrameBuffer<'a>);

impl<'a> Surface for TextureSurface<'a> {
	fn clear_color(&mut self, red: f32, green: f32, blue: f32, alpha: f32) {
		self.0.clear_color(red, green, blue, alpha)
	}

	fn clear_depth(&mut self, value: f32) {
		self.0.clear_depth(value)
	}

	fn clear_stencil(&mut self, value: int) {
		self.0.clear_stencil(value)
	}

	fn get_dimensions(&self) -> (uint, uint) {
		self.0.get_dimensions()
	}

	fn get_depth_buffer_bits(&self) -> Option<u16> {
		self.0.get_depth_buffer_bits()
	}

	fn get_stencil_buffer_bits(&self) -> Option<u16> {
		self.0.get_stencil_buffer_bits()
	}

	fn draw<'b, 'v, V, I, ID, U>(&mut self, vb: V, ib: &I, program: &::Program,
		uniforms: U, draw_parameters: &::DrawParameters)
		where I: ::index_buffer::ToIndicesSource<ID>,
		U: ::uniforms::Uniforms, V: ::vertex_buffer::IntoVerticesSource<'v>
	{
		self.0.draw(vb, ib, program, uniforms, draw_parameters)
	}

	fn get_blit_helper(&self) -> ::BlitHelper {
		self.0.get_blit_helper()
	}
}
