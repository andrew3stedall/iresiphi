terraform {
  required_version = ">= 1.17.2, < 2.0.0"

  required_providers {
    github = {
      source  = "integrations/github"
      version = "~> 6.13.0"
    }
  }
}

provider "github" {
  owner = var.github_owner
}

resource "github_actions_variable" "x86_runner" {
  repository    = var.github_repository
  variable_name = "CI_X86_RUNNER"
  value         = var.x86_runner
}

resource "github_workflow_repository_permissions" "ci" {
  repository                       = var.github_repository
  default_workflow_permissions     = "read"
  can_approve_pull_request_reviews = false
}

