# Meetily's prepared test Mac: one layer on Cirrus's sequoia base image,
# in the layered style of cirruslabs/macos-image-templates.
packer {
  required_plugins {
    tart = { version = ">= 1.21.0", source = "github.com/cirruslabs/tart" }
  }
}

variable "vm_name" { type = string }
variable "model_dir" { type = string }

source "tart-cli" "golden" {
  vm_base_name   = "ghcr.io/cirruslabs/macos-sequoia-base:latest"
  vm_name        = var.vm_name
  cpu_count      = 4
  memory_gb      = 8
  headless       = true
  ssh_username   = "admin"
  ssh_password   = "admin"
  ssh_timeout    = "180s"
  run_extra_args = ["--no-audio"]
}

build {
  sources = ["source.tart-cli.golden"]

  # Files made on the host at build time; `generated = true` skips Packer's up-front existence check.
  provisioner "shell-local" {
    inline = ["bash clips/make-clips.sh build/clips", "bash zoom-standin/build.sh build/zoom-standin"]
  }
  provisioner "shell" {
    inline = ["mkdir -p ~/clips ~/testenv-ui ~/model-staging '/Users/admin/Library/Application Support/com.meetily.ai/models'"]
  }
  provisioner "file" {
    source      = "build/clips/"
    destination = "/Users/admin/clips/"
    generated   = true
  }
  provisioner "file" {
    source      = "build/zoom-standin/"
    destination = "/Users/admin/"
    generated   = true
  }
  # scp rejects a destination with a space, so stage the model first and move it in.
  provisioner "file" {
    source      = var.model_dir
    destination = "/Users/admin/model-staging/"
  }
  provisioner "shell" {
    inline = ["mv /Users/admin/model-staging/* '/Users/admin/Library/Application Support/com.meetily.ai/models/' && rmdir /Users/admin/model-staging"]
  }
  provisioner "file" {
    source      = "ui/"
    destination = "/Users/admin/testenv-ui/"
  }
  provisioner "file" {
    source      = "launch-app.sh"
    destination = "/Users/admin/launch-app.sh"
  }
  provisioner "shell" {
    inline = ["sudo mv '/Users/admin/Zoom Stand-in.app' /Applications/"]
  }
  provisioner "shell" {
    script = "provision/audio.sh"
  }
  provisioner "shell" {
    script = "provision/tcc.sh"
  }
  provisioner "shell" {
    script = "provision/ui.sh"
  }
}
