# Exemplos de devolutivas aprovadas

> Este arquivo contém devolutivas reais que aprovei e que servem de referência
> para a IA imitar tom, estrutura e nível de detalhe. A aplicação também
> injeta automaticamente os exemplos mais recentes aprovados via histórico —
> este arquivo é o "núcleo curado" para casos especialmente bons.
>
> Adicione novos exemplos copiando o bloco abaixo e preenchendo.

---

## Exemplo 1 — Correção de cálculo fiscal

**Entrada bruta (o que eu digitei):**
> corrigi o bug de cálculo de ICMS na NF de devolução, estava somando em vez
> de subtrair. liberei na 2.54.5 do dia 08/05

**Devolutiva final aprovada:**

## O que foi alterado/corrigido
Corrigido o cálculo do ICMS em notas fiscais de devolução. A rotina estava
**somando** o valor do imposto à base ao invés de **subtrair**, resultando em
valores divergentes nos relatórios fiscais.

## Caminho da solução
`Guardian > Movimentações > Notas Fiscais > Emissão > Devolução`

## Release/versão
`v2.54.5 — 08/05/2026`

## Necessidade de atualização
Atualização padrão do sistema. Não é necessário trocar executável.

## Forma de validação
Sugiro validar emitindo uma NF de devolução em ambiente de homologação e
conferir o valor de ICMS no relatório de apuração. O valor agora deve refletir
o débito correto.

## Solução paliativa ou definitiva
Definitiva.

---

## Exemplo 2 — Implementação de novo parâmetro

**Entrada bruta:**
> criei o parâmetro pra travar edição de pedido após faturamento, em natureza
> de operação. release 2.54.6 dia 15/05. sem script

**Devolutiva final aprovada:**

## O que foi alterado/corrigido
Implementado novo parâmetro para impedir a edição de pedidos de venda após o
faturamento. Quando habilitado, o sistema bloqueia qualquer alteração no
pedido a partir do momento em que existir nota fiscal vinculada.

## Caminho da solução
`Guardian > Cadastros > Compra/Venda > Natureza de Operação > Aba Parâmetros`

## Novo parâmetro ou permissão
**Parâmetro:** "Bloquear edição de pedido após faturamento"
**Localização:** Cadastro de natureza de operação, aba Parâmetros.
**Uso correto:** marcar para naturezas em que o operador não deve mais
alterar o pedido depois que a NF foi emitida. Deixar desmarcado mantém o
comportamento anterior.

## Release/versão
`v2.54.6 — 15/05/2026`

## Necessidade de atualização
Atualização padrão do sistema. Não é necessário trocar executável.

## Forma de validação
Habilitar o parâmetro em uma natureza de operação de teste, gerar um pedido
com essa natureza, faturar a NF e tentar editar o pedido. O sistema deve
exibir mensagem de bloqueio.

## Solução paliativa ou definitiva
Definitiva.

---

<!--
Modelo para adicionar novos exemplos:

## Exemplo N — Título descritivo

**Entrada bruta:**
> ...

**Devolutiva final aprovada:**

## O que foi alterado/corrigido
...
-->
