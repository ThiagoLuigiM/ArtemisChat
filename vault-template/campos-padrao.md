# Valores frequentes e padrões recorrentes

> Constantes que se repetem em muitas devolutivas. A IA usa este arquivo
> para preencher campos comuns sem que eu precise digitar todo vez.

## Versão atual em produção
<!-- Atualize a cada release -->
- **Release atual:** `v2.54.6 — 15/05/2026`
- **Próxima release prevista:** `v2.54.7 — 22/05/2026`

## Sistemas/módulos que mais aparecem
- **Guardian** — ERP principal
- **Guardian Servidor** — backend/serviços
- **Artemis** — módulo fiscal
- **Artemis Servidor** — backend fiscal

## Caminhos recorrentes (exemplos)
- `Guardian > Cadastros > Compra/Venda > Natureza de Operação > Aba Parâmetros`
- `Guardian > Movimentações > Notas Fiscais > Emissão`
- `Guardian > Configurações > Parâmetros do Sistema`
- `Artemis > Apurações > ICMS > Devolução`

## Frases-modelo aprovadas
Use exatamente estas frases quando o caso for padrão. Ajuste apenas valores.

**Para correção sem necessidade de update de executável:**
> A correção foi liberada na release `vX.Y.Z — dd/mm/aaaa` e está disponível
> automaticamente após atualização padrão do sistema. Não é necessário trocar
> executável.

**Para correção que exige troca de executável:**
> A correção foi liberada na release `vX.Y.Z — dd/mm/aaaa`. É necessário
> aplicar a atualização do sistema e substituir o executável `<nome>.exe`
> na pasta de instalação do cliente.

**Para validação por parte do suporte:**
> Sugiro validar reproduzindo o cenário relatado no ticket: `<passo a passo>`.
> O resultado esperado é `<comportamento correto>`.

**Para cenário não simulado:**
> O cenário exato do cliente não foi reproduzido em ambiente de desenvolvimento.
> Os testes realizados foram: `<lista>`. Recomendo que o suporte valide com o
> cliente em ambiente de homologação antes de aplicar em produção.

## Glossário interno
- **N1** — primeiro nível de suporte ao cliente
- **N2** — suporte técnico avançado
- **DEV** — equipe de desenvolvimento
- **Homologa** — ambiente de homologação do cliente
