# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Initial release: OpenAPI 3.0 → installable Rust CLI generator
- Auto-normalization: media types, response variants, schemaless bodies, enum collisions, property name collisions, unsupported schema types
- OpenAPI 3.1 downgrade pass
- Auth: none, bearer, apikey, http basic, oauth2-as-bearer
- progenitor 0.14 integration via in-process library calls
- Smoke-tested specs: petstore, Plausible, PokeAPI
