variable "github_owner" {
  description = "Owner of the existing GitHub repository."
  type        = string
  default     = "andrew3stedall"
}

variable "github_repository" {
  description = "Name of the existing GitHub repository (without its owner)."
  type        = string
  default     = "iresiphi"
}

variable "x86_runner" {
  description = "GitHub-hosted Linux x86 runner for ARM-incompatible CI workloads."
  type        = string
  default     = "ubuntu-24.04"

  validation {
    condition     = contains(["ubuntu-22.04", "ubuntu-24.04"], var.x86_runner)
    error_message = "Choose an explicit supported x86 runner: ubuntu-22.04 or ubuntu-24.04."
  }
}

