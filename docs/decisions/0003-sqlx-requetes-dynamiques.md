# 0003 — sqlx sans requêtes vérifiées à la compilation

**Contexte.** La question Q4 retenait `sqlx` pour ses requêtes vérifiées à la
compilation (`query!`). Ces macros demandent une base au moment de compiler,
ou des fichiers `.sqlx` régénérés avec `sqlx-cli` à chaque changement de schéma.

**Décision.** `sqlx` est gardé (asynchrone, migrations intégrées), mais avec
les fonctions `sqlx::query` non vérifiées. Les requêtes sont couvertes par les
tests de `crates/ot-server/tests/store.rs`.

**Conséquences.** Compilation et CI sans base ni outil supplémentaire. Une
erreur SQL apparaît aux tests et non à la compilation. Revenir aux macros reste
possible sans changer le schéma.
