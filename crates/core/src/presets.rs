//! Runner images and framework presets.
//!
//! devpush loads these from a remote registry catalog; Runway embeds the
//! catalog (the runner images are public on ghcr.io/devpushhq until the
//! self-hosted registry work lands). Phase 2 adds framework detection.

/// Runner slug → image.
pub fn runner_image(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "python-3.12" => "ghcr.io/devpushhq/runner-python-3.12:1.0.1",
        "pypy-3.11" => "ghcr.io/devpushhq/runner-pypy-3.11:1.0.1",
        "node-20" => "ghcr.io/devpushhq/runner-node-20:1.0.1",
        "bun-1.3" => "ghcr.io/devpushhq/runner-bun-1.3:1.0.1",
        "go-1.25" => "ghcr.io/devpushhq/runner-go-1.25:1.0.1",
        "php-fpm-8.3" => "ghcr.io/devpushhq/runner-php-fpm-8.3:1.0.4",
        "php-fpm-node-8.3" => "ghcr.io/devpushhq/runner-php-fpm-node-8.3:1.0.4",
        "frankenphp-8.3" => "ghcr.io/devpushhq/runner-frankenphp-8.3:1.0.2",
        "frankenphp-node-8.3" => "ghcr.io/devpushhq/runner-frankenphp-node-8.3:1.0.2",
        "frankenphp-node-worker-8.3" => "ghcr.io/devpushhq/runner-frankenphp-node-worker-8.3:1.0.2",
        _ => return None,
    })
}

pub struct Preset {
    pub slug: &'static str,
    pub name: &'static str,
    pub runner: &'static str,
    pub build_command: &'static str,
    pub pre_deploy_command: &'static str,
    pub start_command: &'static str,
    pub port: u16,
}

/// Subset of the catalog for the Phase 1 MVP. Phase 2 adds detection +
/// static output; PHP runners exist for parity but presets wait.
pub const PRESETS: &[Preset] = &[
    Preset {
        slug: "nextjs",
        name: "Next.js",
        runner: "node-20",
        build_command: "npm install && npm run build",
        pre_deploy_command: "",
        start_command: "npm start -- -p ${PORT:-3000} -H 0.0.0.0",
        port: 3000,
    },
    Preset {
        slug: "nodejs",
        name: "Node.js",
        runner: "node-20",
        build_command: "npm install && npm run build",
        pre_deploy_command: "",
        start_command: "npm start",
        port: 8000,
    },
    Preset {
        slug: "python",
        name: "Python",
        runner: "python-3.12",
        build_command: "pip install -r requirements.txt",
        pre_deploy_command: "",
        start_command: "gunicorn -w 3 -b 0.0.0.0:8000 main:app",
        port: 8000,
    },
];

pub fn preset(slug: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.slug == slug)
}
