{{- define "bunker.labels" -}}
app.kubernetes.io/name: {{ .Release.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "bunker.selectorLabels" -}}
app.kubernetes.io/name: {{ .Release.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "bunker.dockerTokenRealm" -}}
https://{{ .Values.ingress.host }}/v2/token
{{- end -}}

{{- define "bunker.publicUrl" -}}
https://{{ .Values.ingress.host }}
{{- end -}}
