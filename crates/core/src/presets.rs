//! Runner images and framework presets.
//!
//! devpush loads these from a remote registry catalog; Runway embeds the
//! catalog (the runner images are public on ghcr.io/devpushhq until the
//! self-hosted registry work lands). Detection mirrors the reference:
//! `any_files`/`all_files`/`none_files`/`package_check` + priority.

/// Runner slug → image.
pub fn runner_image(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "python-3.12" => "ghcr.io/devpushhq/runner-python-3.12:1.0.1",
        "pypy-3.11" => "ghcr.io/devpushhq/runner-pypy-3.11:1.0.1",
        "node-20" => "ghcr.io/devpushhq/runner-node-20:1.0.1",
        // jarvis: official image until a dedicated node-22 runner ships;
        // bookworm (not slim) carries git + a C toolchain.
        "node-22" => "node:22-bookworm",
        "bun-1.3" => "ghcr.io/devpushhq/runner-bun-1.3:1.0.1",
        "go-1.25" => "ghcr.io/devpushhq/runner-go-1.25:1.0.1",
        "php-fpm-8.3" => "ghcr.io/devpushhq/runner-php-fpm-8.3:1.0.4",
        "php-fpm-node-8.3" => "ghcr.io/devpushhq/runner-php-fpm-node-8.3:1.0.4",
        "frankenphp-8.3" => "ghcr.io/devpushhq/runner-frankenphp-8.3:1.0.2",
        "frankenphp-node-8.3" => "ghcr.io/devpushhq/runner-frankenphp-node-8.3:1.0.2",
        "frankenphp-node-worker-8.3" => "ghcr.io/devpushhq/runner-frankenphp-node-worker-8.3:1.0.2",
        // Static deployments: build in a runner container, then serve
        // output_directory through this tiny file server.
        "static-web" => "joseluisq/static-web-server:2-alpine",
        _ => return None,
    })
}

/// `runner_image` with `runner-overrides.json` applied — a file under
/// `data_dir` mapping runner slugs to replacement images, so a
/// self-hosted instance isn't bound to ghcr.io/devpushhq:
///
/// ```json
/// { "runners": { "node-20": { "image": "registry.example.com/node:20",
///                              "enabled": true } } }
/// ```
///
/// `enabled: false` blocks a runner entirely. Read per call — edits take
/// effect without a restart.
pub fn runner_image_resolved(data_dir: &str, slug: &str) -> Option<String> {
    let overrides: Option<serde_json::Value> =
        std::fs::read_to_string(std::path::Path::new(data_dir).join("runner-overrides.json"))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok());
    if let Some(entry) = overrides
        .as_ref()
        .and_then(|o| o.get("runners"))
        .and_then(|r| r.get(slug))
    {
        if entry.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            return None;
        }
        if let Some(img) = entry.get("image").and_then(|v| v.as_str()) {
            return Some(img.to_string());
        }
    }
    runner_image(slug).map(str::to_string)
}

/// Detection rule — mirrors devpush registry `detection` objects.
/// File patterns support a leading `*/` for "in any subdirectory".
pub struct Detection {
    pub priority: i32,
    pub any_files: &'static [&'static str],
    pub all_files: &'static [&'static str],
    pub none_files: &'static [&'static str],
    /// Substring checked against package.json contents (deps + name).
    pub package_check: Option<&'static str>,
}

pub struct Preset {
    pub slug: &'static str,
    pub name: &'static str,
    pub runner: &'static str,
    pub build_command: &'static str,
    pub pre_deploy_command: &'static str,
    pub start_command: &'static str,
    pub port: u16,
    /// When set, the deployment is static: the runner builds, then a
    /// static file server serves this directory (relative to root dir).
    pub output_dir: Option<&'static str>,
    /// Static-mode only: serve index.html on 404 (history-API SPAs).
    pub spa_fallback: bool,
    pub detection: Detection,
}

#[allow(clippy::too_many_arguments)] // table-shape mirrors the catalog
const fn p(
    slug: &'static str,
    name: &'static str,
    runner: &'static str,
    build_command: &'static str,
    pre_deploy_command: &'static str,
    start_command: &'static str,
    port: u16,
    output_dir: Option<&'static str>,
    spa_fallback: bool,
    priority: i32,
    any_files: &'static [&'static str],
    all_files: &'static [&'static str],
    none_files: &'static [&'static str],
    package_check: Option<&'static str>,
) -> Preset {
    Preset {
        slug,
        name,
        runner,
        build_command,
        pre_deploy_command,
        start_command,
        port,
        output_dir,
        spa_fallback,
        detection: Detection {
            priority,
            any_files,
            all_files,
            none_files,
            package_check,
        },
    }
}

pub const PRESETS: &[Preset] = &[
    // -- Frontend (the wedge) ------------------------------------------------
    p(
        "nextjs", "Next.js", "node-20",
        "npm install && npm run build", "",
        "npx next start -H 0.0.0.0 -p ${PORT:-3000}", 3000, None, false,
        110, &["next.config.js", "next.config.mjs", "next.config.ts", "package.json"], &[], &[],
        Some("next"),
    ),
    p(
        "astro-ssr", "Astro (SSR)", "node-20",
        "npm install && npm run build", "",
        "node ./dist/server/entry.mjs", 4321, None, false,
        105, &["astro.config.mjs", "astro.config.ts", "package.json"], &[], &[],
        Some("@astrojs/node"),
    ),
    p(
        "astro", "Astro (static)", "node-20",
        "npm install && npm run build", "",
        "", 80, Some("dist"), false,
        100, &["astro.config.mjs", "astro.config.ts", "package.json"], &[], &[],
        Some("astro"),
    ),
    p(
        "sveltekit", "SvelteKit", "node-20",
        "npm install && npm run build", "",
        "node build", 3000, None, false,
        100, &["svelte.config.js", "package.json"], &[], &[],
        Some("@sveltejs/kit"),
    ),
    p(
        "nuxt", "Nuxt", "node-20",
        "npm install && npm run build", "",
        "node .output/server/index.mjs", 3000, None, false,
        100, &["nuxt.config.ts", "nuxt.config.js", "package.json"], &[], &[],
        Some("nuxt"),
    ),
    p(
        "remix", "Remix", "node-20",
        "npm install && npm run build", "",
        "npm start", 3000, None, false,
        95, &["remix.config.js", "vite.config.ts", "package.json"], &[], &[],
        Some("@remix-run"),
    ),
    p(
        "vite", "Vite / SPA", "node-20",
        "npm install && npm run build", "",
        "", 80, Some("dist"), true,
        60, &["vite.config.ts", "vite.config.js", "package.json"], &[], &[],
        Some("vite"),
    ),
    p(
        "angular", "Angular", "node-20",
        "npm install && npm run build && mkdir -p .rw-out && (cp -r dist/*/browser/* .rw-out/ 2>/dev/null || cp -r dist/* .rw-out/)", "",
        "", 80, Some(".rw-out"), true,
        80, &["angular.json"], &["package.json"], &[],
        Some("@angular/core"),
    ),
    p(
        "solidstart", "SolidStart", "node-20",
        "npm install && npm run build", "",
        "node .output/server/index.mjs", 3000, None, false,
        80, &["app.config.ts", "app.config.js", "package.json"], &[], &[],
        Some("@solidjs/start"),
    ),
    p(
        "qwik", "Qwik", "node-20",
        "npm install && npm run build", "",
        "", 80, Some("dist"), true,
        70, &["package.json"], &[], &["angular.json"],
        Some("@builder.io/qwik"),
    ),
    p(
        "eleventy", "Eleventy", "node-20",
        "npm install && npx @11ty/eleventy", "",
        "", 80, Some("_site"), false,
        80, &[".eleventy.js", "eleventy.config.js", "eleventy.config.cjs"], &[], &[],
        None,
    ),
    p(
        "gatsby", "Gatsby", "node-20",
        "npm install && npm run build", "",
        "", 80, Some("public"), false,
        85, &["gatsby-config.js", "gatsby-config.ts"], &["package.json"], &[],
        Some("gatsby"),
    ),
    p(
        "docusaurus", "Docusaurus", "node-20",
        "npm install && npm run build", "",
        "", 80, Some("build"), false,
        80, &["docusaurus.config.js", "docusaurus.config.ts"], &["package.json"], &[],
        Some("@docusaurus"),
    ),
    p(
        "hugo", "Hugo", "go-1.25",
        "go install github.com/gohugoio/hugo@latest && hugo --minify", "",
        "", 80, Some("public"), false,
        95, &["hugo.toml", "hugo.yaml", "config.toml"], &[], &[],
        None,
    ),
    // Bare HTML/CSS/JS — no build, served as-is. Last-resort match.
    p(
        "static", "Static", "node-20",
        "", "",
        "", 80, Some("."), false,
        10, &["index.html", "*/index.html"], &[], &["package.json", "go.mod", "requirements.txt", "composer.json"],
        None,
    ),
    // -- Backend (parity with the devpush catalog) ---------------------------
    p(
        "flask", "Flask", "python-3.12",
        "pip install -r requirements.txt", "",
        "gunicorn -w 3 -b 0.0.0.0:8000 main:app", 8000, None, false,
        90, &["requirements.txt", "pyproject.toml", "*/requirements.txt"], &[], &["manage.py"],
        None,
    ),
    p(
        "django", "Django", "python-3.12",
        "pip install -r requirements.txt", "python manage.py migrate --noinput",
        "gunicorn -w 3 -b 0.0.0.0:8000 ${WSGI_MODULE:-config.wsgi}:application", 8000, None, false,
        90, &["manage.py"], &["requirements.txt"], &[],
        Some("django"),
    ),
    p(
        "fastapi", "FastAPI", "python-3.12",
        "pip install -r requirements.txt", "",
        "uvicorn main:app --host 0.0.0.0 --port 8000", 8000, None, false,
        90, &["requirements.txt", "pyproject.toml"], &[], &[],
        Some("fastapi"),
    ),
    p(
        "python", "Python", "python-3.12",
        "pip install -r requirements.txt", "",
        "gunicorn -w 3 -b 0.0.0.0:8000 main:app", 8000, None, false,
        30, &["requirements.txt", "pyproject.toml", "setup.py", "Pipfile", "poetry.lock"],
        &[], &[], None,
    ),
    p(
        "nodejs", "Node.js", "node-20",
        "npm install && npm run build", "",
        "npm start", 8000, None, false,
        40, &["package.json"], &[], &["bun.lockb", "bun.lock"], None,
    ),
    p(
        "nestjs", "NestJS", "node-20",
        "npm install && npm run build", "npm run migration:run 2>/dev/null || true",
        "npm run start:prod", 8000, None, false,
        50, &["nest-cli.json"], &[], &[],
        Some("@nestjs/core"),
    ),
    p(
        "bun", "Bun", "bun-1.3",
        "bun install", "",
        "bun run start", 8000, None, false,
        95, &["bun.lockb", "bun.lock"], &["package.json"], &[], None,
    ),
    p(
        "go", "Go", "go-1.25",
        "go mod download && go build -o app .", "",
        "./app", 8000, None, false,
        100, &["go.mod"], &[], &[], None,
    ),
    p(
        "php", "PHP", "frankenphp-8.3",
        "composer install --no-dev --optimize-autoloader --no-interaction --no-progress", "",
        "frankenphp run --config /etc/caddy/Caddyfile --adapter caddyfile", 8000, None, false,
        50, &["composer.json", "index.php", "*/index.php"], &[], &["artisan"], None,
    ),
    p(
        "laravel", "Laravel", "frankenphp-8.3",
        "composer install --no-dev --optimize-autoloader --no-interaction --no-progress",
        "php artisan migrate --force && php artisan config:cache && php artisan route:cache && php artisan view:cache",
        "frankenphp run --config /etc/caddy/Caddyfile --adapter caddyfile", 8000, None, false,
        100, &["artisan"], &["composer.json"], &[], None,
    ),
];

pub fn preset(slug: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.slug == slug)
}

/// Lockfile → package manager for node-family presets.
pub fn package_manager(files: &[&str]) -> &'static str {
    if file_matches(files, "pnpm-lock.yaml") || file_matches(files, "pnpm-workspace.yaml") {
        "pnpm"
    } else if file_matches(files, "yarn.lock") {
        "yarn"
    } else if file_matches(files, "bun.lock") || file_matches(files, "bun.lockb") {
        "bun"
    } else {
        "npm"
    }
}

/// Rewrite npm-flavored preset commands for the detected package
/// manager. No-op for npm or non-node configs.
pub fn adjust_for_pm(config: &mut serde_json::Value, pm: &str) {
    if pm == "npm" {
        return;
    }
    let Some(obj) = config.as_object_mut() else {
        return;
    };
    let (install, build, start) = match pm {
        "pnpm" => (
            "pnpm install --frozen-lockfile",
            "pnpm run build",
            "pnpm start",
        ),
        "yarn" => ("yarn install --frozen-lockfile", "yarn build", "yarn start"),
        "bun" => ("bun install", "bun run build", "bun start"),
        _ => return,
    };
    for key in ["build_command", "pre_deploy_command", "start_command"] {
        if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
            let new = v
                .replace("npm ci", install)
                .replace("npm install", install)
                .replace("npm run build", build)
                .replace("npm start", start);
            if new != v {
                obj.insert(key.to_string(), new.into());
            }
        }
    }
}

fn file_matches(files: &[&str], pattern: &str) -> bool {
    if let Some(rest) = pattern.strip_prefix("*/") {
        files
            .iter()
            .any(|f| *f == rest || f.ends_with(&format!("/{rest}")))
    } else {
        files.contains(&pattern)
    }
}

/// Pick the highest-priority preset matching the repo file list.
/// `package_json` is the raw package.json content when present.
pub fn detect(files: &[&str], package_json: Option<&str>) -> Option<&'static Preset> {
    PRESETS
        .iter()
        .filter(|p| {
            let d = &p.detection;
            (d.all_files.is_empty() || d.all_files.iter().all(|f| file_matches(files, f)))
                && (d.any_files.is_empty() || d.any_files.iter().any(|f| file_matches(files, f)))
                && d.none_files.iter().all(|f| !file_matches(files, f))
                && d.package_check
                    .is_none_or(|needle| package_json.is_some_and(|pj| pj.contains(needle)))
        })
        .max_by_key(|p| p.detection.priority)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_nextjs() {
        let files = vec!["package.json", "next.config.mjs"];
        let pj = Some(r#"{"dependencies":{"next":"15.0.0","react":"19"}}"#);
        assert_eq!(detect(&files, pj).unwrap().slug, "nextjs");
    }

    #[test]
    fn detects_astro_static() {
        let files = vec!["package.json", "astro.config.mjs"];
        let pj = Some(r#"{"dependencies":{"astro":"5.0.0"}}"#);
        assert_eq!(detect(&files, pj).unwrap().slug, "astro");
    }

    #[test]
    fn astro_ssr_beats_static_when_adapter_present() {
        let files = vec!["package.json", "astro.config.mjs"];
        let pj = Some(r#"{"dependencies":{"astro":"5.0.0","@astrojs/node":"9.0.0"}}"#);
        assert_eq!(detect(&files, pj).unwrap().slug, "astro-ssr");
    }

    #[test]
    fn detects_plain_node() {
        let files = vec!["package.json"];
        assert_eq!(
            detect(&files, Some(r#"{"dependencies":{"express":"4"}}"#))
                .unwrap()
                .slug,
            "nodejs"
        );
    }

    #[test]
    fn bun_lockfile_wins() {
        let files = vec!["package.json", "bun.lock"];
        assert_eq!(detect(&files, Some("{}")).unwrap().slug, "bun");
    }

    #[test]
    fn detects_go() {
        assert_eq!(detect(&["go.mod", "main.go"], None).unwrap().slug, "go");
    }

    #[test]
    fn detects_static_site() {
        assert_eq!(
            detect(&["index.html", "style.css", "app.js"], None)
                .unwrap()
                .slug,
            "static"
        );
    }

    #[test]
    fn package_json_beats_static() {
        let files = vec!["index.html", "package.json"];
        assert_eq!(detect(&files, Some("{}")).unwrap().slug, "nodejs");
    }

    #[test]
    fn detects_angular() {
        let files = vec!["angular.json", "package.json"];
        let pj = Some(r#"{"dependencies":{"@angular/core":"18.0.0"}}"#);
        assert_eq!(detect(&files, pj).unwrap().slug, "angular");
    }

    #[test]
    fn detects_eleventy() {
        let files = vec![".eleventy.js", "package.json", "index.md"];
        assert_eq!(detect(&files, Some("{}")).unwrap().slug, "eleventy");
    }

    #[test]
    fn detects_gatsby() {
        let files = vec!["gatsby-config.js", "package.json"];
        let pj = Some(r#"{"dependencies":{"gatsby":"5.0.0","react":"18"}}"#);
        assert_eq!(detect(&files, pj).unwrap().slug, "gatsby");
    }

    #[test]
    fn detects_docusaurus() {
        let files = vec!["docusaurus.config.ts", "package.json"];
        let pj = Some(r#"{"dependencies":{"@docusaurus/core":"3.0.0"}}"#);
        assert_eq!(detect(&files, pj).unwrap().slug, "docusaurus");
    }

    #[test]
    fn laravel_beats_php() {
        let files = vec!["artisan", "composer.json"];
        assert_eq!(detect(&files, None).unwrap().slug, "laravel");
    }

    #[test]
    fn subdirectory_patterns() {
        let files = vec!["api/requirements.txt"];
        assert!(file_matches(&files, "*/requirements.txt"));
        assert!(!file_matches(&files, "requirements.txt"));
    }

    #[test]
    fn runner_overrides() {
        let dir = std::env::temp_dir().join(format!("runway-preset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();

        // No file → static mapping.
        assert_eq!(
            runner_image_resolved(d, "node-20").as_deref(),
            Some("ghcr.io/devpushhq/runner-node-20:1.0.1")
        );

        std::fs::write(
            dir.join("runner-overrides.json"),
            r#"{"runners": {
                "node-20": {"image": "registry.local/node:20"},
                "bun-1.3": {"enabled": false}
            }}"#,
        )
        .unwrap();
        assert_eq!(
            runner_image_resolved(d, "node-20").as_deref(),
            Some("registry.local/node:20")
        );
        assert_eq!(runner_image_resolved(d, "bun-1.3"), None);
        // Unmentioned slugs pass through.
        assert!(runner_image_resolved(d, "static-web").is_some());

        std::fs::remove_dir_all(&dir).ok();
    }
}
