{{- define "artiferris.labels" -}}
app.kubernetes.io/name: {{ .Release.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "artiferris.selectorLabels" -}}
app.kubernetes.io/name: {{ .Release.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "artiferris.dockerTokenRealm" -}}
https://{{ .Values.ingress.host }}/v2/token
{{- end -}}

{{- define "artiferris.publicUrl" -}}
https://{{ .Values.ingress.host }}
{{- end -}}
