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
