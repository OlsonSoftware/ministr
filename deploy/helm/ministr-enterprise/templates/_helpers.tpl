{{/*
F5.4-c — common naming + labels helpers. Standard Helm idiom from
the `helm create` scaffold; kept minimal because the chart only ships
a Deployment + Service + Secret today.
*/}}

{{/*
Name of the chart instance. Defaults to .Release.Name truncated to 63
chars (k8s label-value limit) but can be overridden via
.Values.nameOverride.
*/}}
{{- define "ministr-enterprise.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Fully-qualified app name — used for resource names. Truncates to 63
chars to fit k8s validation. Includes release name unless
.Values.fullnameOverride is set.
*/}}
{{- define "ministr-enterprise.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{/*
Chart label used in app.kubernetes.io/version.
*/}}
{{- define "ministr-enterprise.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Recommended k8s labels — see
https://kubernetes.io/docs/concepts/overview/working-with-objects/common-labels/.
*/}}
{{- define "ministr-enterprise.labels" -}}
helm.sh/chart: {{ include "ministr-enterprise.chart" . }}
{{ include "ministr-enterprise.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{/*
Selector labels — the subset that's stable across upgrades (so
Deployments can match Pods without rolling).
*/}}
{{- define "ministr-enterprise.selectorLabels" -}}
app.kubernetes.io/name: {{ include "ministr-enterprise.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
