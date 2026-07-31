//! World-space UI labels: the game pushes (text, world position) pairs each
//! frame; the adapter projects them through the active camera and renders
//! them as pooled UI text nodes. Generic engine facility — name tags,
//! waypoints, damage numbers.

use bevy::prelude::*;
use bevy::transform::TransformSystem;
use glam::Vec3 as GVec3;

#[derive(Debug, Clone)]
pub struct WorldLabel {
    pub text: String,
    pub world_pos: GVec3,
    pub color: [f32; 4],
    pub font_size: f32,
}

/// Cleared by the engine each frame; the game re-pushes what it wants shown.
#[derive(Resource, Default)]
pub struct WorldLabels(pub Vec<WorldLabel>);

impl WorldLabels {
    pub fn push(&mut self, text: impl Into<String>, world_pos: GVec3, color: [f32; 4], size: f32) {
        self.0.push(WorldLabel {
            text: text.into(),
            world_pos,
            color,
            font_size: size,
        });
    }
}

#[derive(Component)]
struct LabelPoolNode;

pub(crate) struct WorldLabelPlugin;

impl Plugin for WorldLabelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldLabels>().add_systems(
            PostUpdate,
            project_world_labels.after(TransformSystem::TransformPropagate),
        );
    }
}

#[allow(clippy::type_complexity)]
fn project_world_labels(
    labels: Res<WorldLabels>,
    mut commands: Commands,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut pool: Query<
        (
            &mut Node,
            &mut Text,
            &mut TextFont,
            &mut TextColor,
            &mut Visibility,
        ),
        With<LabelPoolNode>,
    >,
) {
    let Some((camera, cam_transform)) = cameras.iter().find(|(c, _)| c.is_active) else {
        return;
    };

    let mut pool_iter = pool.iter_mut();
    for label in labels.0.iter() {
        let projected = camera
            .world_to_viewport(cam_transform, label.world_pos)
            .ok();
        match pool_iter.next() {
            Some((mut node, mut text, mut font, mut color, mut visibility)) => match projected {
                Some(pos) => {
                    node.left = Val::Px(pos.x + 10.0);
                    node.top = Val::Px(pos.y - 8.0);
                    if text.0 != label.text {
                        text.0 = label.text.clone();
                    }
                    font.font_size = label.font_size;
                    color.0 = Color::srgba(
                        label.color[0],
                        label.color[1],
                        label.color[2],
                        label.color[3],
                    );
                    *visibility = Visibility::Visible;
                }
                None => {
                    *visibility = Visibility::Hidden;
                }
            },
            None => {
                // Grow the pool; positioned properly next frame.
                if let Some(pos) = projected {
                    commands.spawn((
                        LabelPoolNode,
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(pos.x + 10.0),
                            top: Val::Px(pos.y - 8.0),
                            ..Default::default()
                        },
                        Text::new(label.text.clone()),
                        TextFont {
                            font_size: label.font_size,
                            ..Default::default()
                        },
                        TextColor(Color::srgba(
                            label.color[0],
                            label.color[1],
                            label.color[2],
                            label.color[3],
                        )),
                        GlobalZIndex(10),
                    ));
                }
            }
        }
    }
    // Hide any surplus pooled nodes.
    for (_, _, _, _, mut visibility) in pool_iter {
        *visibility = Visibility::Hidden;
    }
}
