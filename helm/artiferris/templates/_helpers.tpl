{{- define "artiferris.labels" -}}
app.kubernetes.io/name: {{ .Release.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "artiferris.selectorLabels" -}}
app.kubernetes.io/name: {{ .Release.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "artiferris.image" -}}
{{- $tag := required "image.tag must be set to a release tag (e.g. --set image.tag=1.4.2)" .Values.image.tag -}}
{{- if .Values.image.digest -}}
{{ .Values.image.repository }}:{{ $tag }}@{{ .Values.image.digest }}
{{- else -}}
{{ .Values.image.repository }}:{{ $tag }}
{{- end -}}
{{- end -}}

{{/* A moving tag would never be refreshed under IfNotPresent. */}}
{{- define "artiferris.pullPolicy" -}}
{{- if eq .Values.image.tag "latest" }}Always{{ else }}{{ .Values.image.pullPolicy }}{{ end -}}
{{- end -}}

{{- define "artiferris.publicUrl" -}}
https://{{ .Values.ingress.host }}
{{- end -}}

{{/* Comma-separated CIDRs with the blanks stripped; empty when nothing is left. */}}
{{- define "artiferris.trustedProxyIps" -}}
{{- $items := list -}}
{{- range splitList "," (toString (default "" .Values.artiferris.trustedProxyIps)) -}}
{{- with trim . -}}{{- $items = append $items . -}}{{- end -}}
{{- end -}}
{{- join "," $items -}}
{{- end -}}

{{- define "artiferris.validateDatabase" -}}
{{- $ext := .Values.externalDatabase -}}
{{- if .Values.postgres.enabled -}}
{{- if or $ext.existingSecret $ext.url -}}
{{- fail "externalDatabase is set but postgres.enabled is true: set postgres.enabled=false to use the external database." -}}
{{- end -}}
{{- else if and $ext.existingSecret $ext.url -}}
{{- fail "externalDatabase.existingSecret and externalDatabase.url are both set: keep only one." -}}
{{- else if not (or $ext.existingSecret $ext.url) -}}
{{- fail "postgres.enabled is false but no external database is configured: set externalDatabase.existingSecret (e.g. a CloudNativePG <cluster>-app Secret, key uri) or externalDatabase.url." -}}
{{- end -}}
{{- end -}}

{{/* Where the app reads DATABASE_URL: the release Secret, or the external database's own Secret. */}}
{{- define "artiferris.databaseSecretName" -}}
{{- if or .Values.postgres.enabled .Values.externalDatabase.url -}}
{{ .Release.Name }}-secrets
{{- else -}}
{{ .Values.externalDatabase.existingSecret }}
{{- end -}}
{{- end -}}

{{- define "artiferris.databaseSecretKey" -}}
{{- if or .Values.postgres.enabled .Values.externalDatabase.url -}}
DATABASE_URL
{{- else -}}
{{ required "externalDatabase.existingSecretKey must be set" .Values.externalDatabase.existingSecretKey }}
{{- end -}}
{{- end -}}
