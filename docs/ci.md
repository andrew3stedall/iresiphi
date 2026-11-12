# CI and processor architecture

[CI](../.github/workflows/ci.yml) runs on pushes, pull requests (including forks),
and manual dispatch. The `x86 CI` job uses a GitHub-hosted Linux x86 machine and
checks `uname -m` before running tools. ARM development machines do not need to
emulate x86 to run this job. GitHub documents `ubuntu-24.04` as an x64 runner in
its [runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).

Every workload that requires x86 must run through GitHub Actions on an x86
runner. This includes the preview's Linux x86_64 wheels, native dependency audits,
and minimum-glibc container tests. ARM-compatible builds, tests and processing may
run locally using native ARM tools and artifacts. A local x86 emulator is not a
substitute for the required workflow evidence.

## Required passes: ARM and x86

A change is accepted, and a release gate is met, only when it passes on both architectures at the same commit:

- **ARM (aarch64):** the full Rust suite (`cargo test --locked`), the Python suites on CPython 3.12 and 3.13 against installed wheels, the Rust/Python parity golden, and the 100,000-line workload. These run locally on native ARM.
- **x86 (x86_64):** the same suites and workload, plus the manylinux_2_28 wheel build, `auditwheel`, and clean-image installs. These run in the GitHub Actions x86 job.

A pass on one architecture does not stand in for the other. Record each pass with its commit; evidence for an earlier commit does not carry forward.

The repository includes the Rust/Python invoice-review preview. CI validates
Terraform and runs the Rust suite, Python packaging checks and workload evidence
on x86. TypeScript and ARM release wheels remain deferred by the preview scope.

The [x86 evidence record](measurements/ci-x86.md) identifies verified runs,
artifact contents and the immutable manylinux image. Workflow artifacts are
specific to a commit; a job definition alone does not establish a passing result.

## Terraform-managed settings

[`infra/github`](../infra/github) manages settings on the existing repository:

- `CI_X86_RUNNER`, defaulting to `ubuntu-24.04`. The workflow uses that same
  default before Terraform is applied. Terraform restricts the value to explicit
  x86 runner labels; the runtime architecture check catches manual misconfiguration.
- Read-only default workflow token permissions, with workflow approval of pull
  requests disabled. The CI workflow itself only requests `contents: read`.

GitHub provisions and disposes of the machines. Terraform does not create a
cloud VM or self-hosted runner. No cloud credentials are needed. Provider details:
[Actions variables](https://registry.terraform.io/providers/integrations/github/latest/docs/resources/actions_variable)
and [workflow permissions](https://registry.terraform.io/providers/integrations/github/latest/docs/resources/workflow_repository_permissions).

## Apply repository settings

Use Terraform 1.17.2 (the version pinned in CI). The provider supports ARM, so
these commands can run on an ARM development machine. Supply `GITHUB_TOKEN`
through your shell or credential manager with repository Administration and
Variables write permissions. The workflow's read-only `GITHUB_TOKEN` is not
sufficient to manage these settings. Terraform uses the provider's environment
authentication; credentials do not belong in `.tf` files.

If `gh` is already authenticated with repository admin access, reuse its login
instead of creating another token:

```sh
export GITHUB_TOKEN="$(gh auth token)"
```

```sh
terraform -chdir=infra/github init
terraform -chdir=infra/github plan -out=settings.tfplan
terraform -chdir=infra/github apply settings.tfplan
```

The defaults target `andrew3stedall/iresiphi`. Override `github_owner`,
`github_repository`, or `x86_runner` with `TF_VAR_...` environment variables or an
ignored `.tfvars` file when needed.

If these settings already exist, import them into state before planning:

```sh
terraform -chdir=infra/github import github_workflow_repository_permissions.ci iresiphi
# Only if CI_X86_RUNNER already exists:
terraform -chdir=infra/github import github_actions_variable.x86_runner iresiphi:CI_X86_RUNNER
```

Keep the generated state between applies. This bootstrap uses local state, which
is ignored by Git along with plans and local inputs. Use one designated operator
until a shared, locking backend is configured. CI does not apply settings or store
state on disposable runners. Repository administrators apply the reviewed plan;
PR jobs, including fork PRs, require no management credentials.

## Validate locally

```sh
terraform fmt -check -recursive
terraform -chdir=infra/github init -backend=false -lockfile=readonly
terraform -chdir=infra/github validate
```

These checks need network access to download the locked provider but do not need
GitHub authentication. The committed provider lock file includes Linux ARM64,
Linux AMD64, and macOS ARM64 checksums. See HashiCorp's
[validation documentation](https://developer.hashicorp.com/terraform/cli/commands/validate)
for the distinction between offline validation and a plan against live settings.
