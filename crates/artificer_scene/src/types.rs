use crate::mesh::MeshData;
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Scene node id, allocated by [`crate::SceneGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct NodeId(pub u64);

/// Registered mesh geometry id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MeshId(pub u64);

/// Registered texture id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct TextureId(pub u64);

/// How a texture is sampled.
///
/// `Nearest` is the default because the packs this engine is built to consume
/// are atlas-based: neighbouring swatches sit pixels apart on one page, and
/// bilinear filtering bleeds one into the next along every UV seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TextureSampling {
    #[default]
    Nearest,
    Linear,
}

/// Whether a texture's bytes are colour or DATA.
///
/// Getting this wrong is invisible in the asset and visible only in the
/// lighting. A normal map decoded as sRGB has every component pushed through
/// a gamma curve, so its vectors no longer point where they should and the
/// surface lights as though lit from somewhere else -- subtly, in a way that
/// reads as "the model looks a bit off" rather than as a bug. Base colour is
/// sRGB; normal, metallic-roughness and occlusion are linear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TextureColorSpace {
    /// Colour, gamma-encoded. Base colour and emissive.
    #[default]
    Srgb,
    /// Raw values. Normal, metallic-roughness, occlusion.
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TransformDesc {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl TransformDesc {
    pub const IDENTITY: TransformDesc = TransformDesc {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    pub fn from_translation_rotation(translation: Vec3, rotation: Quat) -> Self {
        Self {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Compose: `self` is the parent, `child` is local.
    pub fn mul(&self, child: &TransformDesc) -> TransformDesc {
        TransformDesc {
            translation: self.translation + self.rotation * (self.scale * child.translation),
            rotation: self.rotation * child.rotation,
            scale: self.scale * child.scale,
        }
    }

    /// Unit vector the node is facing (-Z convention, matching the renderer).
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    /// A transform positioned at `eye` looking at `target`.
    pub fn looking_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let forward = (target - eye).normalize_or_zero();
        let rotation = if forward.length_squared() > 0.0 {
            Quat::from_mat4(&Mat4::look_to_rh(Vec3::ZERO, forward, up).inverse())
        } else {
            Quat::IDENTITY
        };
        Self {
            translation: eye,
            rotation,
            scale: Vec3::ONE,
        }
    }
}

impl Default for TransformDesc {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AlphaModeDesc {
    #[default]
    Opaque,
    Blend,
    /// Additive blending — glows, holograms, energy effects.
    Add,
}

/// PBR-ish material description. `emissive` components may exceed 1.0 to
/// drive bloom on HDR cameras.
///
/// Every field is `#[serde(default)]` so a material can be authored in a data
/// file by naming only what differs from the default. Without it, overriding
/// one colour in an import manifest means spelling out all seven fields.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
// `default` alone would let a misspelled field ("roughnes") silently become
// the default value, which is the exact failure deny_unknown_fields exists to
// prevent everywhere else in the import vocabulary.
#[serde(default, deny_unknown_fields)]
pub struct MaterialDesc {
    /// Albedo texture. Multiplies `base_color`, so an untextured material and
    /// a white-tinted textured one are the same expression.
    ///
    /// The pack stores this as a string id and the loader resolves it to a
    /// [`TextureId`] at load, which is why baking never has to invent handle
    /// numbers that would differ between runs.
    pub base_color_texture: Option<TextureId>,
    /// Tangent-space normal map.
    ///
    /// This is where a hard-surface asset's detail actually lives. Panel
    /// gaps, rivets, vents and recesses are almost never geometry on a
    /// game-budget hull -- they are relief in this map, and without it the
    /// same asset reads as a smooth shape with lines PAINTED on it. A
    /// generated ship with a 4K normal map bound looks like its concept art;
    /// the same ship with only base colour looks like a toy.
    pub normal_texture: Option<TextureId>,
    /// Combined metallic-roughness map, glTF convention: roughness in G,
    /// metallic in B.
    ///
    /// One texture rather than two because that is how glTF packs it and how
    /// every generator emits it; splitting them here would mean recombining
    /// them for the renderer anyway. Scales `metallic` and `roughness`.
    pub metallic_roughness_texture: Option<TextureId>,
    /// Baked ambient occlusion, in R.
    ///
    /// Cheap contact darkening. It is what makes a panel gap read as a gap
    /// rather than as a dark line, and it costs nothing at runtime because
    /// the shadowing is already baked.
    pub occlusion_texture: Option<TextureId>,
    /// Emissive map, multiplied by `emissive`.
    ///
    /// This is how a surface glows in *places* rather than all over: lava
    /// cracks, city lights on a night side, lit windows. A uniform `emissive`
    /// colour makes the whole body radiate evenly, which reads as a lamp
    /// shade, not a world. Set `emissive` to white (or an HDR multiplier) to
    /// use the map's own colours.
    ///
    /// `serde(skip)`, deliberately: packs serialize `MaterialDesc` with
    /// POSITIONAL postcard, so a new serialized field invalidates every
    /// baked v3 `.apack` in the wild. Emissive maps are runtime-generated
    /// (procgen) today and the pack pipeline has no emissive slot anyway;
    /// when packs learn emissive, add it there and bump PACK_FORMAT_VERSION.
    #[serde(skip)]
    pub emissive_texture: Option<TextureId>,
    pub sampling: TextureSampling,
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub unlit: bool,
    pub alpha: AlphaModeDesc,
    pub double_sided: bool,
    /// Whether this surface is rendered into shadow maps.
    ///
    /// Defaults to true, because most geometry should occlude light. Turn it
    /// off for things that are *depicting* a light source rather than being
    /// lit by one -- a star billboard, a sky dome, a glowing volume. A
    /// directional light's shadow pass is orthographic along the light axis,
    /// so a sun sphere modelled at the light's own position sits between that
    /// light and the entire scene and shadows all of it.
    pub casts_shadows: bool,
}

impl Default for MaterialDesc {
    fn default() -> Self {
        Self {
            casts_shadows: true,
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            sampling: TextureSampling::Nearest,
            base_color: [0.8, 0.8, 0.8, 1.0],
            metallic: 0.0,
            roughness: 0.6,
            emissive: [0.0, 0.0, 0.0],
            unlit: false,
            alpha: AlphaModeDesc::Opaque,
            double_sided: false,
        }
    }
}

impl MaterialDesc {
    pub fn color(r: f32, g: f32, b: f32) -> Self {
        Self {
            base_color: [r, g, b, 1.0],
            ..Default::default()
        }
    }

    pub fn metal(r: f32, g: f32, b: f32, roughness: f32) -> Self {
        Self {
            base_color: [r, g, b, 1.0],
            metallic: 1.0,
            roughness,
            ..Default::default()
        }
    }

    /// Emissive surface; `intensity` > 1 blooms on HDR cameras.
    /// A surface that depicts a light source.
    ///
    /// Non-casting by construction: something that glows is being seen, not
    /// blocking the view of something else.
    pub fn glow(r: f32, g: f32, b: f32, intensity: f32) -> Self {
        Self {
            base_color: [0.0, 0.0, 0.0, 1.0],
            emissive: [r * intensity, g * intensity, b * intensity],
            casts_shadows: false,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LightDesc {
    Directional {
        color: [f32; 3],
        illuminance: f32,
        shadows: bool,
    },
    Point {
        color: [f32; 3],
        intensity: f32,
        range: f32,
        shadows: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BloomDesc {
    /// 0.0 = off, ~0.15 natural, higher = dreamy.
    pub intensity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ToneMapDesc {
    None,
    Reinhard,
    /// Filmic display transform (TonyMcMapface under the Bevy adapter).
    #[default]
    Filmic,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraDesc {
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
    pub hdr: bool,
    pub bloom: Option<BloomDesc>,
    pub tonemapping: ToneMapDesc,
}

impl Default for CameraDesc {
    fn default() -> Self {
        Self {
            fov_y_degrees: 60.0,
            near: 0.1,
            far: 20_000.0,
            hdr: true,
            bloom: Some(BloomDesc { intensity: 0.15 }),
            tonemapping: ToneMapDesc::Filmic,
        }
    }
}

/// Global rendering environment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentDesc {
    pub clear_color: [f32; 4],
    pub ambient_color: [f32; 3],
    pub ambient_brightness: f32,
}

impl Default for EnvironmentDesc {
    fn default() -> Self {
        Self {
            clear_color: [0.0, 0.0, 0.0, 1.0],
            ambient_color: [1.0, 1.0, 1.0],
            ambient_brightness: 80.0,
        }
    }
}

/// A planet's scattering atmosphere, rendered as an additive shell.
///
/// The shell *mesh* only provides screen coverage — the shader does analytic
/// ray-sphere intersection against the radii below, so the mesh should be a
/// sphere of `atmosphere_radius` around the same origin as the planet. The
/// planet's world-space centre is taken from the node transform (the adapter
/// keeps the shader in sync when the node moves), which is why there is no
/// `center` field here.
///
/// Colour comes from `rayleigh`: per-channel scattering strength. Light that
/// scatters toward the camera is tinted by these coefficients; light that
/// *passes through* has them subtracted, which is what makes the terminator
/// ring glow in the complementary colour — an Earth-blue atmosphere gets
/// orange sunsets for free.
///
/// Limitations by design (the cheap tier): single scattering, and the camera
/// is assumed OUTSIDE the shell. Both are the right trade for orbital
/// scenery; ground-level skies are a different feature.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AtmosphereDesc {
    /// Radius of the solid body, in world units.
    pub planet_radius: f32,
    /// Outer radius of the scattering shell. ~1.05–1.15 × planet radius
    /// reads as thin/earthlike; more reads as thick/soupy.
    pub atmosphere_radius: f32,
    /// Per-channel Rayleigh scattering coefficients, per world unit.
    /// Direction sets the colour; magnitude sets the density.
    pub rayleigh: [f32; 3],
    /// Altitude (world units) over which Rayleigh density falls by 1/e.
    pub rayleigh_scale_height: f32,
    /// Mie (haze) scattering coefficient, per world unit. Colourless.
    pub mie: f32,
    /// Altitude (world units) over which Mie density falls by 1/e.
    pub mie_scale_height: f32,
    /// Mie phase anisotropy, 0 = isotropic, →1 = tight forward glare.
    pub mie_g: f32,
    /// World-space position of the light source the shell scatters.
    pub sun_position: [f32; 3],
    /// Scattered-light intensity multiplier (HDR; drives bloom).
    pub sun_intensity: f32,
}

impl Default for AtmosphereDesc {
    fn default() -> Self {
        Self {
            planet_radius: 1.0,
            atmosphere_radius: 1.1,
            // Earth-like blue: strengths ∝ 1/λ⁴, scaled for a unit planet.
            rayleigh: [5.5, 13.0, 28.4],
            rayleigh_scale_height: 0.02,
            mie: 2.0,
            mie_scale_height: 0.01,
            mie_g: 0.76,
            sun_position: [0.0, 0.0, 1.0e6],
            sun_intensity: 22.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeKind {
    Mesh {
        mesh: MeshId,
        material: MaterialDesc,
    },
    /// Scattering shell over a planet (see [`AtmosphereDesc`]).
    Atmosphere {
        mesh: MeshId,
        atmosphere: AtmosphereDesc,
    },
    Light(LightDesc),
    Camera(CameraDesc),
    Group,
}

/// The serializable mutation stream consumed by render adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SceneCommand {
    AddMesh {
        id: MeshId,
        data: MeshData,
    },
    /// Upload an encoded image (PNG). Kept encoded rather than decoded to raw
    /// pixels so the bytes travel as they came out of the bake, and the
    /// renderer's own decoder does the work.
    AddTexture {
        id: TextureId,
        png: Vec<u8>,
        sampling: TextureSampling,
        color_space: TextureColorSpace,
    },
    Spawn {
        id: NodeId,
        parent: Option<NodeId>,
        transform: TransformDesc,
        kind: NodeKind,
    },
    SetTransform {
        id: NodeId,
        transform: TransformDesc,
    },
    SetVisible {
        id: NodeId,
        visible: bool,
    },
    SetMaterial {
        id: NodeId,
        material: MaterialDesc,
    },
    Despawn {
        id: NodeId,
    },
    SetActiveCamera {
        id: NodeId,
    },
    SetEnvironment {
        env: EnvironmentDesc,
    },
    // New variants go at the END: postcard encodes the variant INDEX, so an
    // insertion above renumbers everything below it and old recordings
    // decode as the wrong command with a plausible payload.
    /// Forget a registered mesh (see ADR-0005). DEREGISTRATION, not
    /// destruction: nodes already spawned keep their own handles and render
    /// on; the GPU asset is freed once the last of them despawns. After
    /// this, new spawns naming the id get the "unknown mesh" warning.
    RemoveMesh {
        id: MeshId,
    },
    /// Forget a registered texture. Same semantics as [`RemoveMesh`]:
    /// materials already built keep the image alive until their nodes go.
    RemoveTexture {
        id: TextureId,
    },
}
