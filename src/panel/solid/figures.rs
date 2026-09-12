//! The mascots, each built from solid parts.

use super::*;

/// Characters typed on a screen per second.
pub(super) const TYPING_RATE: f32 = 9.0;
/// Seconds the finished text stays up before the screen clears and typing
/// starts over.
pub(super) const TYPING_PAUSE: f32 = 2.5;

pub(super) const WHITE: Color = Color::Rgb(250, 250, 250);
pub(super) const DARK: Color = Color::Rgb(28, 28, 36);
pub(super) const RED: Color = Color::Rgb(255, 80, 80);

/// Two eyes looking out of the front of a figure at `y`, `x` apart, on the
/// surface at depth `z`: white balls with the pupils turned the way the
/// figure is looking, or a closed line while it blinks.
pub(super) fn eyes(gaze: Gaze, x: f32, y: f32, z: f32, r: f32) -> Vec<Part> {
    if gaze.blink {
        return vec![
            mark(Shape::Sphere { c: v(-x, y, z), r }, WHITE, '-'),
            mark(Shape::Sphere { c: v(x, y, z), r }, WHITE, '-'),
        ];
    }
    let pupil = gaze.look.scale(r * 0.85);
    vec![
        mark(Shape::Sphere { c: v(-x, y, z), r }, WHITE, 'O'),
        mark(Shape::Sphere { c: v(x, y, z), r }, WHITE, 'O'),
        mark(
            Shape::Sphere {
                c: v(-x, y, z).add(pupil),
                r: r * 0.38,
            },
            DARK,
            '#',
        ),
        mark(
            Shape::Sphere {
                c: v(x, y, z).add(pupil),
                r: r * 0.38,
            },
            DARK,
            '#',
        ),
    ]
}

/// The glyph a lidless eye is drawn in: its own, or a line while it blinks.
pub(super) fn lid(gaze: Gaze, open: char) -> char {
    if gaze.blink { '-' } else { open }
}

/// Kodee, the Kotlin mascot: a purple body with two pointed ears, a black
/// screen for a face with two big white eyes, and noodle arms and legs. The
/// right arm waves with `t`.
pub fn kodee(t: f32, gaze: Gaze) -> Vec<Part> {
    let purple = Color::Rgb(125, 82, 255);
    let wave = (t * 2.0).sin();
    let mut parts = vec![
        part(
            Shape::RoundBox {
                c: v(0.0, 0.1, 0.0),
                h: v(0.95, 0.75, 0.3),
                r: 0.2,
            },
            purple,
        ),
        part(
            Shape::Prism {
                c: v(-0.62, 0.9, 0.0),
                w: 0.42,
                h: 0.6,
                d: 0.3,
            },
            purple,
        ),
        part(
            Shape::Prism {
                c: v(0.62, 0.9, 0.0),
                w: 0.42,
                h: 0.6,
                d: 0.3,
            },
            purple,
        ),
        mark(
            Shape::RoundBox {
                c: v(0.0, 0.05, 0.42),
                h: v(0.7, 0.45, 0.04),
                r: 0.06,
            },
            DARK,
            '#',
        ),
        part(
            Shape::Capsule {
                a: v(-1.0, -0.2, 0.0),
                b: v(-1.45, -0.9, 0.15),
                r: 0.11,
            },
            purple,
        ),
        part(
            Shape::Capsule {
                a: v(1.0, -0.1, 0.0),
                b: v(1.5, 0.5 + 0.4 * wave, 0.2),
                r: 0.11,
            },
            purple,
        ),
        part(
            Shape::Capsule {
                a: v(-0.4, -0.8, 0.0),
                b: v(-0.5, -1.65, 0.1),
                r: 0.12,
            },
            purple,
        ),
        part(
            Shape::Capsule {
                a: v(0.4, -0.8, 0.0),
                b: v(0.5, -1.65, 0.1),
                r: 0.12,
            },
            purple,
        ),
        part(
            Shape::Capsule {
                a: v(-0.5, -1.65, 0.1),
                b: v(-0.75, -1.65, 0.35),
                r: 0.12,
            },
            purple,
        ),
        part(
            Shape::Capsule {
                a: v(0.5, -1.65, 0.1),
                b: v(0.25, -1.65, 0.35),
                r: 0.12,
            },
            purple,
        ),
    ];
    parts.extend(eyes(gaze, 0.3, 0.1, 0.5, 0.2));
    parts
}

/// Ferris the crab: a flat orange body with a spiky back, eyes on top,
/// six legs, and two claws that wave with `t`.
pub fn ferris(t: f32, gaze: Gaze) -> Vec<Part> {
    let orange = Color::Rgb(247, 96, 20);
    let wave = (t * 2.0).sin() * 0.35;
    let mut parts = vec![part(
        Shape::Ellipsoid {
            c: v(0.0, -0.3, 0.0),
            r: v(1.35, 0.7, 0.95),
        },
        orange,
    )];
    for i in 0..5 {
        let x = -0.8 + 0.4 * i as f32;
        parts.push(part(
            Shape::Prism {
                c: v(x, 0.2, 0.0),
                w: 0.16,
                h: 0.4,
                d: 0.16,
            },
            orange,
        ));
    }
    for side in [-1.0f32, 1.0] {
        parts.push(part(
            Shape::Capsule {
                a: v(side * 1.2, -0.3, 0.2),
                b: v(side * 1.75, 0.1 + wave, 0.3),
                r: 0.13,
            },
            orange,
        ));
        parts.push(part(
            Shape::Ellipsoid {
                c: v(side * 1.95, 0.2 + wave, 0.3),
                r: v(0.38, 0.3, 0.25),
            },
            orange,
        ));
        for (i, x) in [0.45f32, 0.85, 1.2].into_iter().enumerate() {
            let z = 0.5 - 0.35 * i as f32;
            parts.push(part(
                Shape::Capsule {
                    a: v(side * x, -0.7, z),
                    b: v(side * (x + 0.3), -1.45, z + 0.2),
                    r: 0.1,
                },
                orange,
            ));
        }
    }
    parts.extend(eyes(gaze, 0.45, -0.05, 0.75, 0.19));
    parts
}

/// The Go gopher: a tall rounded blue body, round ears, huge eyes, a nose
/// and two front teeth, and arms that swing with `t`.
pub fn gopher(t: f32, gaze: Gaze) -> Vec<Part> {
    let blue = Color::Rgb(0, 190, 230);
    let swing = (t * 1.5).sin() * 0.25;
    let mut parts = vec![
        part(
            Shape::RoundBox {
                c: v(0.0, -0.15, 0.0),
                h: v(0.7, 1.0, 0.45),
                r: 0.45,
            },
            blue,
        ),
        part(
            Shape::Sphere {
                c: v(-0.85, 1.05, -0.1),
                r: 0.25,
            },
            blue,
        ),
        part(
            Shape::Sphere {
                c: v(0.85, 1.05, -0.1),
                r: 0.25,
            },
            blue,
        ),
        part(
            Shape::Sphere {
                c: v(0.0, 0.1, 0.78),
                r: 0.16,
            },
            Color::Rgb(235, 200, 160),
        ),
        mark(
            Shape::RoundBox {
                c: v(0.0, -0.22, 0.8),
                h: v(0.17, 0.12, 0.03),
                r: 0.02,
            },
            WHITE,
            '#',
        ),
        part(
            Shape::Capsule {
                a: v(-0.95, -0.5, 0.0),
                b: v(-1.35, -1.1 + swing, 0.3),
                r: 0.13,
            },
            blue,
        ),
        part(
            Shape::Capsule {
                a: v(0.95, -0.5, 0.0),
                b: v(1.35, -1.1 - swing, 0.3),
                r: 0.13,
            },
            blue,
        ),
        part(
            Shape::Ellipsoid {
                c: v(-0.45, -1.6, 0.25),
                r: v(0.35, 0.15, 0.4),
            },
            blue,
        ),
        part(
            Shape::Ellipsoid {
                c: v(0.45, -1.6, 0.25),
                r: v(0.35, 0.15, 0.4),
            },
            blue,
        ),
    ];
    parts.extend(eyes(gaze, 0.4, 0.5, 0.75, 0.32));
    parts
}

/// Duke, the Java mascot: a dark triangle with a white lower half and a red
/// nose, waving one arm with `t`.
pub fn duke(t: f32) -> Vec<Part> {
    let dark = Color::Rgb(120, 120, 140);
    let wave = (t * 2.0).sin() * 0.4;
    vec![
        part(
            Shape::Prism {
                c: v(0.0, -1.2, 0.0),
                w: 1.3,
                h: 2.7,
                d: 0.45,
            },
            dark,
        ),
        part(
            Shape::RoundBox {
                c: v(0.0, -0.75, 0.2),
                h: v(0.75, 0.5, 0.3),
                r: 0.25,
            },
            WHITE,
        ),
        mark(
            Shape::Sphere {
                c: v(0.0, 0.05, 0.5),
                r: 0.2,
            },
            RED,
            '@',
        ),
        part(
            Shape::Capsule {
                a: v(-0.7, -0.4, 0.0),
                b: v(-1.35, -1.0, 0.2),
                r: 0.1,
            },
            dark,
        ),
        part(
            Shape::Capsule {
                a: v(0.7, -0.4, 0.0),
                b: v(1.4, 0.4 + wave, 0.2),
                r: 0.1,
            },
            dark,
        ),
        part(
            Shape::Ellipsoid {
                c: v(-0.4, -1.4, 0.3),
                r: v(0.3, 0.14, 0.35),
            },
            WHITE,
        ),
        part(
            Shape::Ellipsoid {
                c: v(0.4, -1.4, 0.3),
                r: v(0.3, 0.14, 0.35),
            },
            WHITE,
        ),
    ]
}

/// A python: coils of blue and yellow, a raised head that sways, eyes that
/// blink, and a tongue that flicks with `t`.
pub fn snake(t: f32, gaze: Gaze) -> Vec<Part> {
    let blue = Color::Rgb(70, 140, 200);
    let yellow = Color::Rgb(255, 212, 59);
    let flick = if (t * 3.0).sin() > 0.0 { 0.4 } else { 0.0 };
    let sway = (t * 1.3).sin() * 0.15;
    let head = v(0.45 + sway, 0.85, 0.4);
    vec![
        part(
            Shape::Torus {
                c: v(0.0, -1.35, 0.0),
                big: 0.95,
                small: 0.3,
            },
            blue,
        ),
        part(
            Shape::Torus {
                c: v(0.1, -0.85, 0.0),
                big: 0.7,
                small: 0.27,
            },
            yellow,
        ),
        part(
            Shape::Torus {
                c: v(0.15, -0.4, 0.0),
                big: 0.45,
                small: 0.24,
            },
            blue,
        ),
        part(
            Shape::Capsule {
                a: v(0.3, -0.3, 0.1),
                b: v(head.x, head.y - 0.25, head.z - 0.1),
                r: 0.22,
            },
            yellow,
        ),
        part(
            Shape::Ellipsoid {
                c: head,
                r: v(0.45, 0.3, 0.38),
            },
            blue,
        ),
        mark(
            Shape::Capsule {
                a: v(head.x + 0.45, head.y - 0.05, head.z + 0.05),
                b: v(head.x + 0.45 + flick, head.y - 0.15, head.z + 0.1),
                r: 0.07,
            },
            RED,
            '~',
        ),
        mark(
            Shape::Sphere {
                c: v(head.x - 0.15, head.y + 0.12, head.z + 0.3),
                r: 0.09,
            },
            WHITE,
            lid(gaze, 'O'),
        ),
        mark(
            Shape::Sphere {
                c: v(head.x + 0.15, head.y + 0.12, head.z + 0.3),
                r: 0.09,
            },
            WHITE,
            lid(gaze, 'O'),
        ),
    ]
}

/// A monitor with `code` being typed out on its coloured screen, its stand,
/// and a light that blinks with `t`. The text types out, sits a while, then
/// clears and starts again.
pub fn monitor(t: f32, screen: Color, code: &'static [&'static str]) -> Vec<Part> {
    let grey = Color::Rgb(170, 170, 190);
    let led = if (t * 2.0).sin() > 0.0 {
        Color::Rgb(90, 255, 120)
    } else {
        Color::Rgb(40, 90, 50)
    };
    let total = Screen::len(code);
    let cycle = total as f32 / TYPING_RATE + TYPING_PAUSE;
    let typed = ((t % cycle) * TYPING_RATE) as usize;
    let face = Shape::RoundBox {
        c: v(0.0, 0.35, 0.27),
        h: v(1.3, 0.8, 0.03),
        r: 0.02,
    };
    vec![
        part(
            Shape::RoundBox {
                c: v(0.0, 0.3, 0.0),
                h: v(1.5, 1.0, 0.15),
                r: 0.1,
            },
            grey,
        ),
        Part {
            shape: face,
            color: screen,
            glyph: Some(' '),
            screen: Some(Screen {
                c: v(0.0, 0.35, 0.27),
                h: v(1.3, 0.8, 0.03),
                lines: code,
                typed: typed.min(total),
                cursor_on: (t * 4.0).sin() > 0.0,
            }),
        },
        mark(
            Shape::Sphere {
                c: v(1.25, -0.55, 0.27),
                r: 0.07,
            },
            led,
            '*',
        ),
        part(
            Shape::Capsule {
                a: v(0.0, -0.7, 0.0),
                b: v(0.0, -1.4, 0.0),
                r: 0.15,
            },
            grey,
        ),
        part(
            Shape::RoundBox {
                c: v(0.0, -1.5, 0.0),
                h: v(0.8, 0.08, 0.5),
                r: 0.05,
            },
            grey,
        ),
    ]
}

/// A robot: a boxy head on a boxy body, cyan eyes that blink, an antenna
/// whose light blinks with `t`, and arms that swing.
pub fn robot(t: f32, gaze: Gaze) -> Vec<Part> {
    let steel = Color::Rgb(160, 170, 200);
    let cyan = Color::Rgb(80, 230, 255);
    let swing = (t * 1.5).sin() * 0.3;
    let light = if (t * 2.5).sin() > 0.0 {
        RED
    } else {
        Color::Rgb(90, 40, 40)
    };
    vec![
        part(
            Shape::RoundBox {
                c: v(0.0, 0.75, 0.0),
                h: v(0.8, 0.55, 0.55),
                r: 0.1,
            },
            steel,
        ),
        part(
            Shape::RoundBox {
                c: v(0.0, -0.6, 0.0),
                h: v(0.95, 0.7, 0.5),
                r: 0.1,
            },
            steel,
        ),
        part(
            Shape::Capsule {
                a: v(0.0, 1.3, 0.0),
                b: v(0.0, 1.7, 0.0),
                r: 0.05,
            },
            steel,
        ),
        mark(
            Shape::Sphere {
                c: v(0.0, 1.8, 0.0),
                r: 0.12,
            },
            light,
            '*',
        ),
        mark(
            Shape::Sphere {
                c: v(-0.32, 0.8, 0.5),
                r: 0.16,
            },
            cyan,
            lid(gaze, 'o'),
        ),
        mark(
            Shape::Sphere {
                c: v(0.32, 0.8, 0.5),
                r: 0.16,
            },
            cyan,
            lid(gaze, 'o'),
        ),
        mark(
            Shape::RoundBox {
                c: v(0.0, 0.4, 0.55),
                h: v(0.4, 0.06, 0.02),
                r: 0.01,
            },
            cyan,
            '=',
        ),
        part(
            Shape::Capsule {
                a: v(-1.05, -0.1, 0.0),
                b: v(-1.3, -1.0 + swing, 0.2),
                r: 0.13,
            },
            steel,
        ),
        part(
            Shape::Capsule {
                a: v(1.05, -0.1, 0.0),
                b: v(1.3, -1.0 - swing, 0.2),
                r: 0.13,
            },
            steel,
        ),
        part(
            Shape::RoundBox {
                c: v(-0.45, -1.55, 0.1),
                h: v(0.3, 0.15, 0.4),
                r: 0.05,
            },
            steel,
        ),
        part(
            Shape::RoundBox {
                c: v(0.45, -1.55, 0.1),
                h: v(0.3, 0.15, 0.4),
                r: 0.05,
            },
            steel,
        ),
    ]
}
