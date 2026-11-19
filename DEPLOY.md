# Deploying Tomo for free

Three free services, no credit card needed for any of them:

| Part | Service | Why |
|---|---|---|
| Database | **Neon** (already set up) | Postgres, scales to zero |
| API (Rust) | **Render** free web service | Runs the Docker image from this repo |
| Website | **Cloudflare Pages** | Free static hosting, unlimited bandwidth |

GitHub Actions builds the website and puts the result on a branch called `gh-pages`;
Cloudflare Pages serves that branch. No API tokens are involved anywhere.

> **Free-tier behaviour to expect.** Render's free service sleeps after ~15 minutes without
> traffic. The first visit afterwards takes 30-60 seconds to wake it, and open chat connections
> drop and reconnect by themselves. Neon also sleeps when idle and wakes in a second or two.
> That is fine for a portfolio project, but mention it in your README so visitors aren't surprised.

## Order matters (each step needs the address from the one before)

### 1. Deploy the API on Render
1. Create a free account at <https://render.com> (sign in with GitHub).
2. **New → Blueprint**, pick the `tomo` repository. Render reads `render.yaml`.
3. It asks for the values marked `sync: false`. Fill them in:
   | Variable | Value |
   |---|---|
   | `DATABASE_URL` | the same Neon connection string you use locally |
   | `DISCORD_CLIENT_ID` / `DISCORD_CLIENT_SECRET` | from your Discord app |
   | `FRONTEND_URL`, `ALLOWED_ORIGINS`, `PUBLIC_API_URL` | put `https://placeholder.invalid` for now; you'll fix them in step 4 |
4. Click **Apply**. The first build takes several minutes (it compiles Rust).
5. When it's live, note the address, e.g. `https://tomo-api.onrender.com`.
   Check `https://<that address>/health` shows `{"status":"ok","database":true}`.

**Never add `DEV_AUTH`** on Render. Without it, the username login is switched off.

### 2. Build the website
1. In GitHub open the repo → **Settings → Secrets and variables → Actions → Variables tab →
   New repository variable**.
2. Name: `TOMO_API_URL`, value: your Render address from step 1 (no trailing slash).
3. **Actions → Build website → Run workflow**. When it finishes, a `gh-pages` branch exists.

### 3. Host the website on Cloudflare Pages
1. Create a free account at <https://dash.cloudflare.com> and open **Workers & Pages**.
2. **Create → Pages → Connect to Git**, choose the `tomo` repository.
3. Set **Production branch** to `gh-pages`. Leave the **build command empty** and set the
   **output directory** to `/`. (The branch already contains the finished site.)
4. Deploy. You get an address like `https://tomo.pages.dev`.

### 4. Tell the API where the website is
In Render → your service → **Environment**, set:
| Variable | Value |
|---|---|
| `FRONTEND_URL` | `https://tomo.pages.dev` (your real address, no trailing slash) |
| `ALLOWED_ORIGINS` | the same address |
| `PUBLIC_API_URL` | your Render address, e.g. `https://tomo-api.onrender.com` |

Render restarts the service automatically.

### 5. Add the live redirect to Discord
Discord developer portal → your app → **OAuth2 → Redirects → Add Redirect**:
```
https://tomo-api.onrender.com/auth/discord/callback
```
(use your own Render address). Keep the `localhost` one too. Click **Save Changes**.

### 6. Check it
Open your Cloudflare address → **Sign in → Continue with Discord**. You should land on the
profile page.

## Updating later
Push to `main`. Render rebuilds the API by itself, and the website workflow rebuilds the site
when website or shared code changes. Database changes are migrations in `migrations/`,
and they run automatically when the API starts.

## Troubleshooting
| Symptom | Likely cause |
|---|---|
| The website loads but says "API offline" | `TOMO_API_URL` was wrong or missing when the site was built. Fix the variable and re-run **Build website**. |
| Browser console shows a CORS error | `ALLOWED_ORIGINS` doesn't exactly match the website address (scheme included, no trailing slash). |
| Discord says "Invalid OAuth2 redirect_uri" | The redirect in step 5 is missing, has a typo, or `PUBLIC_API_URL` doesn't match your Render address. |
| After Discord approves, you land on the login page with an error | Read the message on that page. "expired" means it took over 10 minutes; just try again. |
| First request is very slow | Render woke from sleep (see the note at the top). |
| Render build fails | Open the build log. The Dockerfile has not been run on this author's machine, so a first failure is possible; fixes are usually one line. |

## Notes on safety
- Secrets live only in Render's dashboard and your local `.env.local`. Neither is in git.
- `DEV_AUTH` must stay unset in production, or anyone could sign in as any username.
- Chat messages are stored in the database and are readable by whoever runs it. Don't promise
  users end-to-end privacy.
- Reports are saved in the `reports` table. There is no admin screen yet, so review them in the
  Neon SQL editor until one exists.
