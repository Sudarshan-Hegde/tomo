# How we work

`main` is production. Everything on `main` is live (after its checks pass), so nothing reaches `main`
without going through a pull request.

```
feature branch  ->  pull request  ->  CI passes  ->  merge to main  ->  live
                                       (fmt, clippy, tests)      Render redeploys the API
                                                                 GitHub Actions republishes the site
```

## Making a change

1. **Branch from an up-to-date `main`.** One feature or fix per branch.
   ```
   git switch main && git pull
   git switch -c feature/short-description        # or fix/..., chore/..., docs/...
   ```
2. **Commit in small, meaningful steps.** Message format (Conventional Commits):
   ```
   feat: show shared servers on swipe cards
   fix: stop typing indicator getting stuck after a reconnect
   chore: update dependencies
   docs: explain the invite flow
   ```
   The first line says *what changed*, in the present tense, under about 70 characters. The body (optional)
   says *why*.
3. **Check locally before pushing** (CI runs exactly these):
   ```
   cargo fmt --all
   cargo clippy -p tomo-api -p tomo-shared --all-targets -- -D warnings
   cargo clippy -p tomo-web --target wasm32-unknown-unknown -- -D warnings
   cargo test -p tomo-shared -p tomo-api
   ```
4. **Open a pull request** against `main` and fill in the template. Read your own diff first; it is the
   cheapest review there is.
5. **Merge when CI is green.** Use "Squash and merge" so `main` reads as one commit per feature. Delete the
   branch.
6. **Watch it go live.** Render redeploys the API; the "Build and publish website" workflow republishes the
   site and then checks `/health`. If that check fails, look at it straight away.

## Rules that keep production safe

- **Never push directly to `main`.** (Branch protection enforces this.)
- **Database changes are new, numbered files in `migrations/`.** Never edit one that has already run, because
  it has already been applied to the live database. Write them so they work on a database that already has
  data (add columns with defaults, backfill, then tighten).
- **Migrations run automatically when the API starts.** That means a bad migration breaks the deploy. Test it
  against a copy first (a Neon branch is free and instant).
- **No secrets in git, ever.** Keys live in `.env.local` (local) and Render's dashboard (production). A new
  setting goes into `.env.example`, `render.yaml` and `DEPLOY.md` in the same pull request that needs it.
- **`DEV_AUTH` is never set in production.**
- **Breaking API changes** (renaming a field, removing an endpoint) must keep working for the website that is
  currently live. Ship the server side first, then the website, then remove the old behaviour in a later change.

## If something breaks in production

1. **Roll back first, investigate second.** In Render: your service -> Deploys -> pick the last good deploy ->
   *Rollback*. For the website: revert the merge commit on `main` (`git revert -m 1 <merge-sha>`) through a
   pull request, and the site republishes.
2. **Then fix forward** on a branch, with a test that would have caught it.
3. **Database migrations are not rolled back by a rollback.** If a migration was the problem, write a new
   migration that undoes it.

## Environments

| | Where | Database | Discord app |
|---|---|---|---|
| Local | your machine | Neon `production` branch today (create a dev branch when real users arrive) | same app, `localhost` redirect |
| Production | Render + Cloudflare Pages | Neon | same app, live redirect |

> **Heads up:** local development currently shares the production database. That is fine while there are no
> real users, but before launch create a separate Neon branch for development and point `.env.local` at it.
