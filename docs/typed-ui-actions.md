# Use typed UI actions

Use `ActionQueue<M>` and `App::with_actions` to send UI commands to your update
function. Keep application data in ordinary state fields; let cssimpler own the
queue and its synchronization.

## 1. Define an action enum

Give each command a variant. Put any associated data in that variant.

```rust
#[derive(Clone)]
enum Action {
    Increment,
    Reset,
    SetCount(u64),
}
```

Use one enum for commands with and without payloads. You do not need bit positions
or a separate payload queue. Actions must implement `Send + 'static` for
`App::with_actions`. Derive `Clone` when you want to use `emit`; `emit` also
requires `Sync`.

## 2. Construct the app with an action-aware update and view

Replace `App::new(state, &sheet, update, view)` with
`App::with_actions(state, &sheet, update, view)`.

Add an action slice to your update callback:

```rust
fn update(
    count: &mut u64,
    actions: &[Action],
    _frame: cssimpler::renderer::FrameInfo,
) -> cssimpler::app::Invalidation {
    for action in actions {
        match action {
            Action::Increment => *count += 1,
            Action::Reset => *count = 0,
            Action::SetCount(value) => *count = *value,
        }
    }
    cssimpler::app::Invalidation::Clean
}
```

Process actions in slice order. The slice borrows the current batch and is valid
for this callback invocation. Clone payloads only when your state needs to own
them after the callback returns.

Return your normal invalidation for changes unrelated to the actions. A nonempty
batch automatically requests a structural view refresh, even when you return
`Invalidation::Clean`. Empty batches do not automatically refresh the view.

## 3. Connect buttons to the queue

Accept `&ActionQueue<Action>` as the second view argument. Use `emit` to create a
reusable no-argument callback:

```rust
fn view(
    count: &u64,
    actions: &cssimpler::ActionQueue<Action>,
) -> cssimpler::core::Node {
    cssimpler::ui! {
        <div>
            <p>{format!("Count: {count}")}</p>
            <button onclick_shared={actions.emit(Action::Increment)}>Increment</button>
            <button onclick_shared={actions.emit(Action::Reset)}>Reset</button>
            <button onclick_shared={actions.emit(Action::SetCount(10))}>Set to 10</button>
        </div>
    }
}
```

Use `push` from custom callbacks or worker threads:

```rust
let sender = actions.clone();
let callback = move || sender.push(Action::SetCount(42));
```

Use `emit_with` when a callback receives a value that should become a payload:

```rust
let set_count = actions.emit_with(Action::SetCount);
set_count(42);
```

Match the mapping function's argument to the event handler's actual argument type.
For example, convert text input to a number in the mapping function before
constructing `Action::SetCount`.

Cloning the queue does not require `Action: Clone`. Sending with `push` or
`emit_with` does not require `Action: Clone + Sync`.

## 4. Run the app

Combine the enum, update, and view above with:

```rust
fn main() -> anyhow::Result<()> {
    let sheet = cssimpler::style::parse_stylesheet(
        "button { padding: 12px; margin: 8px; }",
    )?;
    cssimpler::app::App::with_actions(0_u64, &sheet, update, view)
        .run(cssimpler::renderer::WindowConfig::new("Actions", 480, 240))?;
    Ok(())
}
```

To try the included example, run:

```sh
cargo run --example actions
```

Keep `.with_continuous_updates(true)` when your simulation needs updates at the
configured frame cadence. Action delivery itself wakes an idle window and does
not require continuous polling.

## 5. Migrate an existing bitflag inbox

1. Convert each `ACT_*` constant into an action enum variant.
2. Move payload-bearing operations into the same enum.
3. Replace `fetch_or(...)` callbacks with `actions.emit(...)` or `actions.push(...)`.
4. Replace the atomic `swap(0, ...)` and separate payload-queue drain with a loop
   over the update callback's action slice.
5. Remove the application-owned atomic inbox, mutex queue, and manual action-only
   invalidation checks after all producers and consumers have migrated.

Review behavior when migrating:

- Repeated actions are preserved. Two Step clicks enqueue two commands.
- Actions run in enqueue order, rather than the order of bit tests in the old
  update function. Concurrent producers are ordered by their queue insertions.
- Actions enqueued while an update processes its batch wait for the next update
  invocation. A renderer follow-up update may occur within the same displayed frame.
- The queue delivers commands; it does not implement undo history. Keep your
  existing undo/redo state and invoke it from the corresponding action handlers.
- If your solver accepts only one pending step, count queued Step commands in
  your application and submit them as the worker becomes ready. The UI queue
  preserves commands, but cannot change the solver's scheduling policy.

## 6. Choose explicit refresh control when needed

Use `App::new` with a standalone `ActionQueue` if you need fragment-level refreshes
or want your update to decide whether an action changed anything. The
`with_actions` convenience constructor conservatively refreshes the full view
for every nonempty batch.

For a standalone queue, keep one reusable consumer buffer:

```rust
let queue = cssimpler::ActionQueue::new();
let mut batch = Vec::new();

queue.push(42_u64);
queue.drain_into(&mut batch);
for action in &batch {
    // Apply the action and choose an appropriate Refresh or Invalidation.
    let _ = action;
}
batch.clear(); // Required before the next drain_into call.
```

Clone the queue into callbacks and keep the consumer buffer across updates.
Perform action handling outside the queue lock. With this standalone setup, wire
runtime wakeups and invalidation yourself; automatic runtime integration belongs
to `App::with_actions`.

## Understand the storage and performance tradeoff

The queue uses `Arc<Mutex<Vec<M>>>` without new dependencies. The app swaps two
buffers and clears the consumed batch outside the lock. Once both buffers have
enough capacity for subsequent batches, queue buffering needs no further
allocations. Larger batches and owned payloads such as `String` can still allocate.

Treat this as an unbounded UI command queue, not a high-throughput data stream.
It removes the 64-action limit and preserves payloads, order, and duplicates.
It has more overhead than an atomic bitmask; no benchmark establishes it as the
fastest or smallest possible design. Existing `App::new` users retain their
current event handling and invalidation behavior.
