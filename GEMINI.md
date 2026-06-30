# Coding Conventions

- Avoid `Option` / `None` as a default value. If a function takes optional configuration, model the absence as an explicit variant or a builder. `Option<T>` is reserved for genuinely optional data (e.g. a field that may legitimately not be present in the JSON input).
- Never rely on manual steps. If a step is required, add it to the `justfile`.
- The **justfile is the single source of truth** for all build and check commands.

# Rust Code Style

## Types and data

- **Newtype every domain string.** Never pass bare `String` or `&str` across function boundaries for values that represent a specific concept.
- **Enums for any fixed set of values.** Never pass around magic strings or magic integers when there is a fixed set of values. Match exhaustiveness is the point.
- **Structs over tuples** for any return value with more than one field. Named fields beat positional.
- **Prefer `&str` / `&Path` over `&String` / `&PathBuf` in function signatures.** Owned types only when the function takes ownership.
- **Default to immutable.** `let` over `let mut`. `&` over `&mut`.

## Error handling

- **Never `unwrap()` outside `#[cfg(test)]`.** Use `?`, or `expect("reason")` with a short rationale explaining why the invariant holds.
- **No `panic!()` for runtime errors.** Runtime conditions (bad input, missing field) return `None` or `Err`, not a panic.

## Modules and visibility

- **`pub` is opt-in.** Default to private; only mark `pub` what genuinely needs to be.
- **One concept per file.** Split early rather than letting files grow large.

## Lints and formatting

- **`clippy::all` and `clippy::pedantic` at deny** — already configured in `Cargo.toml`. Fix the code, don't silence the lint. Narrow `#[allow(...)]` with a documented reason is acceptable when the lint is a false positive.
- **`cargo fmt` on every commit.**

## Time and dates

- Use `std::time::SystemTime` and work in UTC seconds. Never assume local time.

## Testing

- **Unit tests live in `#[cfg(test)] mod tests`** in the same file as the code under test.
- **No `unwrap()` rule does not apply in tests.** Tests assert; `unwrap()` is fine.
- **Test names describe the scenario, not the function.** `formats_pace_as_green_when_under_threshold`, not `test_fmt_pace`.

## Naming

- **Australian English.** `colour`, `behaviour`, `serialise`, `cancelled`. Applies to variables, comments, commit messages, and doc strings. Quote stdlib identifiers exactly when calling them (`serialize`, `color`) but spell our own in Australian English.
- **`snake_case` for items, `UpperCamelCase` for types, `SCREAMING_SNAKE_CASE` for consts.** Clippy enforces.
- **No abbreviations.** `reset_time`, not `rst`. Internal locals may abbreviate when obvious within a few lines.

## What we are not doing

- **No `unsafe`.**
- **No `Box<dyn Error>` in public APIs.**
- **No `String` for paths.** `PathBuf` / `&Path`.
- **No global mutable state.** No `static mut`. Configuration is passed down from `main` explicitly.

# Committing Code

- Always use `git add` to stage specific files before committing. Never use `git commit -a`.
- When moving or removing files, use `git mv` and `git rm` — never bare `mv` or `rm`.
- Commit messages follow the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/#summary) spec.

# Localisation

- Write in Australian English. All spelling, grammar, idioms and style should reflect this. Applies to documentation, commit messages, code comments, variable names, and error messages.
