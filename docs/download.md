---
title: Download
layout: main
---

## Download

There is a GUI version and a no-GUI version available.

- [noaa-apt 0.9.8 GNU/Linux x86_64 .deb package](https://github.com/martinber/noaa-apt/releases/download/v0.9.8/noaa-apt_0.9.8-1_amd64.deb).

- [noaa-apt 0.9.8 Windows x86_64](https://github.com/martinber/noaa-apt/releases/download/v0.9.8/noaa-apt-0.9.8-x86_64-windows-gnu.zip).

- [noaa-apt 0.9.8 GNU/Linux x86_64 zip](https://github.com/martinber/noaa-apt/releases/download/v0.9.8/noaa-apt-0.9.8-x86_64-linux-gnu.zip).

- [noaa-apt 0.9.8 GNU/Linux x86_64 (no GUI) zip](https://github.com/martinber/noaa-apt/releases/download/v0.9.8/noaa-apt-0.9.8-x86_64-linux-gnu-nogui.zip).

You can download those executables, or old ones from the
[releases page on GitHub](https://github.com/martinber/noaa-apt/releases).

Your options are:

- GNU/Linux:

    - Download .deb package for Debian based distros (Ubuntu, Linux Mint...)

    - Download executable with GUI.

    - Download executable without GUI.

    - [Compile it yourself](./development.html#compilation).

- Windows:

    - Download executable with GUI.

    - [Compile the GUI version yourself from GNU/Linux](./development.html#compilation).

    - Compile the no-gui version, or compile from Windows but I don't know how to
      do it.

- OSX:

  - [Compile it yourself](./development.html#compilation).

- Something else?

    - At least the no-gui version should work everywhere (Raspberry Pi?) because
      there are no dependencies apart from GTK and a handful of pure Rust
      modules.
      [You should compile it yourself though](./development.html#compilation).

## Dependencies

On Windows there aren't any dependencies, on Linux you probably already have
installed what you need:

- GTK+ >= 3.16 (Only for the GUI version)

- glibc >= 2.19

- libgcc

My builds use a statically linked libssl, so you don't need libssl unless you
compiled noaa-apt yourself.
