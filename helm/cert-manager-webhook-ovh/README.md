# cert-manager-webhook-ovh

Lets cert-manager answer DNS-01 challenges on OVH-hosted DNS, to issue a wildcard certificate for `*.artiferris.pro`.
It is a one-time, cluster-wide setup: after it, every organization subdomain gets HTTPS with no extra step.

## 1. Create an OVH API application

Open this link (scoped to read and write on DNS zones, nothing else):

```
https://api.ovh.com/createToken/?GET=/domain/zone/*&PUT=/domain/zone/*&POST=/domain/zone/*&DELETE=/domain/zone/*
```

Name it (for example `artiferris-cert-manager-dns01`) and keep the **Application Key**, **Application Secret** and
**Consumer Key** somewhere safe. Never paste them in a chat or commit them.

## 2. Create the Secret

```bash
kubectl create secret generic ovh-dns01-credentials \
  --namespace cert-manager \
  --from-literal=applicationKey='<Application Key>' \
  --from-literal=applicationSecret='<Application Secret>' \
  --from-literal=applicationConsumerKey='<Consumer Key>'
```

## 3. Install the webhook

```bash
helm repo add cert-manager-webhook-ovh-charts https://aureq.github.io/cert-manager-webhook-ovh/
helm repo update
helm upgrade --install --namespace cert-manager \
  -f helm/cert-manager-webhook-ovh/values.yaml \
  cm-webhook-ovh cert-manager-webhook-ovh-charts/cert-manager-webhook-ovh
kubectl get clusterissuer artiferris-dns01-issuer   # defined in values.yaml, must be READY
```

## 4. Point the ingress at the issuer

Deploy the ArtiFerris chart with `ingress.clusterIssuer: artiferris-dns01-issuer`, `ingress.host: app.artiferris.pro`
and `ingress.wildcardHost: "*.artiferris.pro"` (see `helm/artiferris/values.yaml`), then check the certificate:

```bash
kubectl get certificate -n artiferris
echo | openssl s_client -connect alume.artiferris.pro:443 -servername alume.artiferris.pro 2>/dev/null | openssl x509 -noout -issuer -ext subjectAltName
```

The issuer must be Let's Encrypt, not `TRAEFIK DEFAULT CERT`, and the names must include `app.artiferris.pro` and
`*.artiferris.pro`.
