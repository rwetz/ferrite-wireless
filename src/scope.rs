//! A fixed-range trace of the actual output, including volume and static.
use ferrite_design::prelude::*;
use gpui::{App, Bounds, IntoElement, Pixels, Window, canvas, div, fill, point, px, size};

pub fn scope(samples: Vec<f32>, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let (paper, ink, grid) = (hsla(p.sunken), hsla(p.accent), hsla(p.line));
    div().h(px(100.)).w_full().bg(paper).child(canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _| {
            let center = bounds.top() + bounds.size.height / 2.;
            window.paint_quad(fill(Bounds::new(point(bounds.left(), center), size(bounds.size.width, px(1.))), grid));
            let count = samples.len().max(2);
            let step = bounds.size.width / (count - 1) as f32;
            for (i, pair) in samples.windows(2).enumerate() {
                let y = |v: f32| center - bounds.size.height * 0.45 * v.clamp(-1., 1.);
                let (a, b) = (y(pair[0]), y(pair[1]));
                window.paint_quad(fill(
                    Bounds::new(point(bounds.left() + step * i as f32, a.min(b)), size(step.max(px(1.)), (a - b).abs().max(px(1.)))),
                    ink,
                ));
            }
        },
    ))
}
