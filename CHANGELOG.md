# Changelog

## 0.5.4

### Patch Changes

- Refactor core-resolver

## 0.5.3

### Patch Changes

- Fix import core

## 0.5.2

### Patch Changes

- update readme

## 0.5.1

### Patch Changes

- Remove logs

## 0.5.0

### Minor Changes

- enhance step definition with title and description

## 0.4.0

### Minor Changes

- e672359: scheduler fixes

## 0.2.0

### Minor Changes

- 7f972c3: remove service integration from codebase

## 0.1.2

### Patch Changes

- 47c76ea: Small fix

## 0.1.1

### Patch Changes

- 6d8d1d3: implement dynamic path resolution for Rust core addon
- 8b3a18b: update reamde

## 0.1.0

### Minor Changes

- Initial release of Cronflow v0.1.0
  - Complete workflow automation engine with Rust core
  - TypeScript SDK with comprehensive workflow definitions
  - Webhook and scheduled trigger support
  - Advanced control flow (if/else, parallel execution)
  - Service integration and state management
  - Optimized build system with bundle size analysis
  - Ready for npm publishing with proper package structure

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial project structure with Node.js SDK and Rust core
- Monolith architecture with root-level directories
- TypeScript configuration for SDK and services
- Build scripts for Rust core and TypeScript compilation
- npm publishing configuration and validation
- Semantic release setup with conventional commits

### Changed

- Switched from Nx monorepo to monolith structure for simplicity
- Updated project structure to root-level core, sdk, and services directories

### Fixed

- Resolved TypeScript path mapping issues
- Fixed build script dependencies and execution order
