---
title: "Keys and Identities"
slug: "keys-and-identities"
---

import ThemedImage from '@theme/ThemedImage';
import useBaseUrl from '@docusaurus/useBaseUrl';


# Keys and Identities

This document explains the following key and identity YAML files that are generated during the deployment of a validator node:

- `public-keys.yaml`.
- `private-keys.yaml`.
- `validator-identity.yaml`.
- `validator-full-node-identity.yaml`.

The following command is executed to generate the above key and identity YAMLs. See, for example, [Step 10 while using AWS to deploy the validator node](validator-node/operator/running-validator-node/run-validator-node-using-aws), or in [Step 10 while using GCP](validator-node/operator/running-validator-node/run-validator-node-using-gcp). 

```bash
aptos genesis generate-keys --output-dir ~/$WORKSPACE/keys
```

## public-keys.yaml

### Example

Click below to see an example YAML configuration:
<details>
<summary>public-keys.yaml</summary>

```yaml
---
account_address: a5a643aa695fc5f34927386c8d767cddcc0607933f40c89a7ad78de7804965b8
account_public_key: "0x9ccfc50f334064e1b24455029a5bc1646a2c4dd2b1433de1364470692ba6b99b"
consensus_public_key: "0xa7e8334381d9f80d33d70da543aea22c87fe9862ab7df5cbef9ee11b5285b89c56e0e7a3a78c1561833b2d6fa4d9d4bf"
consensus_proof_of_possession: "0xa51dfd1734e581df99c4c637324ee38c3e48e51c61c1e1dd03bd5a84cf1cd5b2fa00e976b9a9ea0e0908f0d53085318c03f24de3ebf86b07ff883effe0142e0d3f24c7c1e36dd198ea4d8eb6f5c5a2f3a188de22720bd1914a9effa6f595de38"
full_node_network_public_key: "0xa6845691a00d6cfdaa9823c4d12b2b5e13d2ecfdc3049d0f2838c805bfd01633"
validator_network_public_key: "0x71f2642aeaa6cbfacf75663cf14d2f6e9e1bd890f9bc1c96900fd225cce01836"
```
 
</details>

### Description

| public-keys.yaml | Description |
| --- | --- |
| account_address |  |
| account_public_key |  |
| consensus_public_key |  |
| consensus_proof_of_possession |  |
| full_node_network_public_key |  |
| validator_network_public_key |  |

## private-keys.yaml

### Example

Click below to see an example YAML configuration:
<details>
<summary>private-keys.yaml</summary>

    
```yaml
---
account_address: a5a643aa695fc5f34927386c8d767cddcc0607933f40c89a7ad78de7804965b8
account_private_key: "0x80478d60a52f54a88e7095abf48b1f4294a335b30f1066cd73768b9b789e833f"
consensus_private_key: "0x4aedda33ef3fd71243eb2a926307d8826c95b9939f88e753d62d9bc577e99916"
full_node_network_private_key: "0x689c11c6e5405219b5eae1312086c801e3a044946afc74429e5157b46fb65b61"
validator_network_private_key: "0xa03ec46b24f2f1066d7980dc13b4baf722ba60c367e498e47a657ba0815adb58"
```

</details>

### Description

| private-keys.yaml | Description |
| --- | --- |
| account_address |  |
| account_private_key |  |
| consensus_private_key |  |
| full_node_network_private_key |  |
| validator_network_private_key |  |

## validator-identity.yaml

### Example

Click below to see an example YAML configuration:

<details>
<summary>validator-identity.yaml</summary>
    

```yaml
---
account_address: a5a643aa695fc5f34927386c8d767cddcc0607933f40c89a7ad78de7804965b8
account_private_key: "0x80478d60a52f54a88e7095abf48b1f4294a335b30f1066cd73768b9b789e833f"
consensus_private_key: "0x4aedda33ef3fd71243eb2a926307d8826c95b9939f88e753d62d9bc577e99916"
network_private_key: "0xa03ec46b24f2f1066d7980dc13b4baf722ba60c367e498e47a657ba0815adb58"
```

</details>

### Description

| validator-identity.yaml | Description |
| --- | --- |
| account_address |  |
| account_private_key |  |
| consensus_private_key |  |
| network_private_key |  |


## validator-full-node-identity.yaml

### Example

Click below to see an example YAML configuration:

<details>
<summary>validator-full-node-identity.yaml</summary>

    
```yaml
---
account_address: a5a643aa695fc5f34927386c8d767cddcc0607933f40c89a7ad78de7804965b8
network_private_key: "0x689c11c6e5405219b5eae1312086c801e3a044946afc74429e5157b46fb65b61"
```

</details>
    

### Description

| validator-full-node-identity.yaml | Description |
| --- | --- |
| account_address |  |
| network_private_key |  |

## Identity vs keys

See below a diagram that shows how the identities for the validator node and validator fullnode are derived from the private and public keys:

<ThemedImage
alt="Signed Transaction Flow"
sources={{
    light: useBaseUrl('/img/docs/key-yamls.svg'),
    dark: useBaseUrl('/img/docs/key-yamls-dark.svg'),
  }}
/>