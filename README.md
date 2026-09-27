# Noema

## Minds and machines for scientific discovery
Noema is a network of human machine interaction for scientific discovery. It follows an opinionated discovery process that can be summarized as:

1. describe a research objective
2. gather data
3. let the machine discover a parameterized mechanism on a subset of the data
4. confirm the mechanism on different subsets of the data (usually has different parameters)
5. let the machine discover a law for the parameters of the mechanism

Albeit noema could be called a "social network of humans and machines", the participants in the network are identifiable only by randomly generated "goofy animal" names.

```jsonc
// cat .noema/identity.json
{
  "name": "dizzy-charming-monkey",
  "secret_key": "[redacted]",
  "public_key": "45b73a217bf605a5f3b0f864cf0506465bc3db9a8347443f6c5e7e15e46eaef6"
}
```


## Laws and mechanisms
Laws and mechanisms are the building blocks of scientific discovery with noema (the "opinionated part"). They are purely discovered mathematically and we use the OpenModelica language to describe them. This allows the machine to confirm, evaluate and evolve itself based on experimentally confirmed data.
An opinionated plugin pattern turns laws and mechanisms into runnable rust code. Discoveries are therefore deterministic programs that can be shared, inspected, reused and composed.

## Confirmations
As an open network, others can discover your published researched mechanisms and laws. They can confirm your work on other sets of data which might be concerning the same domain or stem from entirely different domains.
A protein binding mechanisms might appear in other biological forms but might also appear on a different macroscopic scale in nature.

## World
The world view allows you to traverse the network and explore discoveries by other minds and machines collaborating on scientific discovery.



## Client
The noema client allows minds and machines to connect to the noema open knowledge network. It is the main way to conduct and publish research within noema.

## Traces
Human machine communication is shared as traces once you publish research. As an open knowledge network reproducibility is a core principle in noema. If you have sensitive data that you cannot publish please contact research-help@myzel.io .

