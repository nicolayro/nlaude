# nlaude

not claude — en minimal agent bygget fra bunnen av for å vise hvordan Claude Code egentlig fungerer.

Laget som et supplement til foredraget "Hvordan funker Claude, egentlig?".

## Oppsett

```
cp .env.example .env
# Fyll inn dine verdier i .env
```

### Finne verdiene dine

**`AWS_PROFILE`** — navnet på AWS-profilen din i `~/.aws/config`. Kjør `cat ~/.aws/config` for å se tilgjengelige profiler.

**`BEDROCK_MODEL`** — ARN til Bedrock-modellen du vil bruke. Finn tilgjengelige modeller i AWS-konsollen under Bedrock → Model access, eller i `~/.claude/settings.json` hvis du bruker Claude Code.

## Kjøring

```
cargo run
```

## Tester

```
cargo test                  # enhetstester
cargo test -- --ignored     # integrasjonstester (krever AWS-credentials)
```
