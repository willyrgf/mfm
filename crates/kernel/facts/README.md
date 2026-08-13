# mfm-facts

`mfm-facts` owns bounded, secret-free prior-fact requests, Store-selected responses, State-emitted
proposal sets, completeness, and frontier values. It has no Store or Runtime dependency and does
not perform provider or State callbacks. A request contains only the canonical source and subject
identity projected from intent; Store fixes the preparation frontier and constructs the response.
Proposal sets are coordinate-free until Store publishes them with a conclusion.
