# nlaude

not claude — en minimal agent bygget fra bunnen av for å vise hvordan Claude Code egentlig fungerer.

Laget som et supplement til foredraget "Hvordan funker Claude, egentlig?".

## Oppsett

```
cp .env.example .env
# Fyll inn dine verdier i .env
```

## Kjøring

```
cargo run
```

## Tester

```
cargo test                  # enhetstester
cargo test -- --ignored     # integrasjonstester (krever AWS-credentials)
```
