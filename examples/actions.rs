//! Run with `cargo run --example actions`.
use cssimpler::{
    ActionQueue,
    app::{App, Invalidation},
    renderer::WindowConfig,
    style::parse_stylesheet,
    ui,
};

#[derive(Clone)]
enum Action {
    Increment,
    Reset,
}

fn main() -> anyhow::Result<()> {
    let sheet = parse_stylesheet(
        "#app { padding: 24px; background: #fafafa; } button { padding: 12px; margin: 8px; }",
    )?;
    App::with_actions(
        0_u64,
        &sheet,
        |count, actions: &[Action], _frame| {
            for action in actions {
                match action {
                    Action::Increment => *count += 1,
                    Action::Reset => *count = 0,
                }
            }
            Invalidation::Clean // Nonempty action batches refresh automatically.
        },
        |count, actions: &ActionQueue<Action>| {
            ui! {
                <div id="app">
                    <p>{format!("Count: {count}")}</p>
                    <button onclick_shared={actions.emit(Action::Increment)}>Increment</button>
                    <button onclick_shared={actions.emit(Action::Reset)}>Reset</button>
                </div>
            }
        },
    )
    .run(WindowConfig::new("Typed actions", 480, 240))?;
    Ok(())
}
