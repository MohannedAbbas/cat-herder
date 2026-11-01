{{/*
Cat Herder helpers.
*/}}

{{/*
Admin password: reuse the existing Secret's password if present (so upgrades are
stable), otherwise use the configured value or generate a random one. This keeps
credentials out of git while still allowing a plain `helm install` to work.
*/}}
{{- define "cat-herder.adminPassword" -}}
{{- $ns := .Values.namespace | default "cat-herder" -}}
{{- $secret := lookup "v1" "Secret" $ns "cat-herder-admin" -}}
{{- if and $secret (index $secret.data "password") -}}
{{- index $secret.data "password" | b64dec -}}
{{- else -}}
{{- .Values.admin.password | default (randAlphaNum 24) -}}
{{- end -}}
{{- end -}}

{{/*
Postgres password: same reuse/configure/generate pattern as the admin password.
*/}}
{{- define "cat-herder.postgresPassword" -}}
{{- $ns := .Values.namespace | default "cat-herder" -}}
{{- $secret := lookup "v1" "Secret" $ns "cat-herder-postgres" -}}
{{- if and $secret (index $secret.data "password") -}}
{{- index $secret.data "password" | b64dec -}}
{{- else -}}
{{- .Values.dependencies.postgres.password | default (randAlphaNum 24) -}}
{{- end -}}
{{- end -}}
