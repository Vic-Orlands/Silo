# AGENTS.md

## Instructions

- Don't add unnecessary comments when coding. Add comments only when they explain complex logic, functions, or components.
- Listen to instructions properly and ask for clarification if you don't have the full perspective.
- Always check the skills directory and pick the best relevant skills before you start coding, debugging, or doing general lookup work.
- No fallback behavior that can mask failures: if the expected behavior cannot be produced, throw/propagate errors or surface a clear failure state. Only add fallbacks when you explicitly request them.

## Git Commit Convention

All commits in this repository must follow **Conventional Commits**.

### Format

```txt
type(scope): short description
```

### Examples

```txt
feat(auth): add Google OAuth login
fix(db): resolve database connection failure
chore(deps): update dependencies
refactor(ui): simplify navbar layout
docs(readme): add local setup steps
test(auth): add login tests
perf(api): improve query response time
```

## Allowed Commit Types

- `feat` — new feature
- `fix` — bug fix
- `chore` — maintenance, dependencies, config, tooling
- `docs` — documentation only
- `style` — formatting, CSS, UI spacing, no logic change
- `refactor` — code restructuring without behavior change
- `test` — adding or updating tests
- `perf` — performance improvement
- `build` — build system or external dependency changes
- `ci` — CI/CD changes
- `revert` — revert a previous commit

## Rules

1. Use lowercase commit types.
2. Use present tense: `add`, not `added`.
3. Keep the subject short and specific.
4. Do not end the subject with a period.
5. Include a scope when useful, such as `auth`, `db`, `ui`, `api`, `docs`, or `config`.
6. One commit should represent one logical change.
7. Do not mix unrelated changes in one commit.
8. Breaking changes must include `!` after the type or scope.
9. Never use vague commit messages like `update`, `fix bug`, `changes`, `wip`, or `work in progress`.

## Breaking Change Examples

```txt
feat(auth)!: change session token format
fix(api)!: remove legacy response fields
```

## Agent Instruction

Agents must not create commits that do not follow this convention.

Before committing, agents should verify that the commit message matches the required format:

```txt
type(scope): short description
```

If a change does not clearly fit one type, prefer the most specific valid type. For example:

- Use `fix` for bug fixes.
- Use `feat` for new user-facing functionality.
- Use `chore` for dependency, tooling, config, or cleanup changes.
- Use `refactor` only when behavior does not change.
