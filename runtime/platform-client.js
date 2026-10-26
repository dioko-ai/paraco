/** Transport-independent, authority-free facade; the service selects all scopes. */
export function datasetClient(call, dataset = 'private') {
  const invoke = (action, body = {}) => call({dataset, action, ...body});
  return Object.freeze({
    records: Object.freeze({
      get: key => invoke('record.get', {key}),
      put: (key, value, expectedVersion) => invoke('record.put', {key, value, expectedVersion}),
      delete: (key, expectedVersion) => invoke('record.delete', {key, expectedVersion}),
    }),
    artifacts: Object.freeze({
      publish: draft => invoke('artifact.publish', {draft}),
      get: id => invoke('artifact.get', {id}),
      list: (query = {}) => invoke('artifact.list', {query}),
      resolve: (reference, execution) => invoke('artifact.resolve', {reference, executionId:execution.executionId, slot:execution.slot}),
      lineage: id => invoke('artifact.lineage', {id}),
    }),
  });
}
