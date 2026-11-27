//! The generators that came with the app until 0.20 (noise, clouds, checkerboard,
//! stripes), now plugins of the official repository: a project saved with one names it
//! by its plugin's id once opened (see [`migrate`]), drawn once that plugin is installed
//! and offered to install until then.

use super::{EffectInstance, Generator, PluginRef};

/// The built-in generators: none but in tests (see [`testing`]).
pub fn all() -> Vec<Box<dyn Generator>> {
    #[cfg(test)]
    return testing::all();
    #[cfg(not(test))]
    Vec::new()
}

/// Each former built-in generator's id, its plugin's id in the official repository and
/// its name.
const MOVED: [(&str, &str, &str); 4] = [
    ("builtin.noise", "noise", "Noise"),
    ("builtin.clouds", "clouds", "Clouds"),
    ("builtin.checkerboard", "checkerboard", "Checkerboard"),
    ("builtin.stripes", "stripes", "Stripes"),
];

/// The first version of those plugins, the same as the built-in generators.
const MOVED_VERSION: &str = "1.0.0";

/// Gives a former built-in generator the id of the same generator from the official
/// repository's plugin, as installed (`<origin>/<plugin>.<plugin>`), with that plugin
/// recorded as for any other; other generators are left as they are. Its settings are
/// the same.
pub fn migrate(inst: &mut EffectInstance) {
    let Some((_, plugin, name)) = MOVED.iter().find(|(id, ..)| *id == inst.effect) else {
        return;
    };
    let origin = super::plugin::origin(super::repository::OFFICIAL);
    inst.effect = format!("{origin}/{plugin}.{plugin}");
    inst.plugin = Some(PluginRef {
        id: (*plugin).into(),
        version: MOVED_VERSION.into(),
    });
    inst.name = Some((*name).into());
}

/// Generators for the tests, as the registry has none of its own.
#[cfg(test)]
pub mod testing {
    use image::RgbaImage;

    use super::*;
    use crate::render::effects::{ParamSpec, Params, Role};

    /// `"test.grains"` ("Grains"): a seed, a size "Grain" and two colors;
    /// `"test.bands"` ("Bands"): bands of two colors, no seed.
    pub fn all() -> Vec<Box<dyn Generator>> {
        let colors = || {
            [
                ParamSpec::color("color1", "Color 1", [0, 0, 0, 255]),
                ParamSpec::color("color2", "Color 2", [255, 255, 255, 255]),
            ]
        };
        let grains = [ParamSpec::float("size", "Grain", 0.05, 10.0, 0.2).unit("mm")]
            .into_iter()
            .chain(colors())
            .chain([
                ParamSpec::int("seed", "Variation", 0, artwork_plugin_sdk::SEED_MAX, 1)
                    .with_role(Role::Seed),
            ]);
        let bands = [ParamSpec::float("size", "Width", 0.2, 200.0, 5.0).unit("mm")]
            .into_iter()
            .chain(colors());
        vec![
            Box::new(Test {
                id: "test.grains",
                name: "Grains",
                params: grains.collect(),
            }),
            Box::new(Test {
                id: "test.bands",
                name: "Bands",
                params: bands.collect(),
            }),
        ]
    }

    struct Test {
        id: &'static str,
        name: &'static str,
        params: Vec<ParamSpec>,
    }

    impl Generator for Test {
        fn id(&self) -> &str {
            self.id
        }
        fn name(&self) -> &str {
            self.name
        }
        fn params(&self) -> &[ParamSpec] {
            &self.params
        }
        /// Vertical bands of the two colors, `size` mm wide.
        fn generate(&self, img: &mut RgbaImage, p: &Params, px_per_mm: f32) -> Result<(), String> {
            let size = p.f32("size").max(1e-3) * px_per_mm;
            let (a, b) = (p.color("color1"), p.color("color2"));
            for (x, _, px) in img.enumerate_pixels_mut() {
                px.0 = if ((x as f32 / size) as u32).is_multiple_of(2) {
                    a
                } else {
                    b
                };
            }
            Ok(())
        }
    }

    #[test]
    fn former_built_in_generators_name_their_plugin() {
        let mut inst = EffectInstance {
            effect: "builtin.clouds".into(),
            enabled: true,
            params: [("seed".into(), serde_json::Value::from(7))].into(),
            links: Default::default(),
            plugin: None,
            name: None,
        };
        let before = inst.params.clone();
        migrate(&mut inst);
        let origin =
            crate::render::effects::plugin::origin(crate::render::effects::repository::OFFICIAL);
        assert_eq!(inst.effect, format!("{origin}/clouds.clouds"));
        assert_eq!(inst.plugin.as_ref().unwrap().id, "clouds");
        assert_eq!(inst.name.as_deref(), Some("Clouds"));
        assert_eq!(inst.params, before);
        // Others stay.
        let mut other = EffectInstance {
            effect: "o/dots".into(),
            ..inst.clone()
        };
        let kept = other.clone();
        migrate(&mut other);
        assert_eq!(other, kept);
    }
}
