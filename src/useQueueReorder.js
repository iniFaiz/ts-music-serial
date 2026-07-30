import { nextTick, onUnmounted, ref, watch } from 'vue';

export function useQueueReorder(queueLength, moveItem) {
  const dragIndex = ref(-1);
  const overIndex = ref(-1);
  const listContainer = ref(null);
  const disableQueueTransition = ref(false);
  const keyMap = new WeakMap();
  let keySequence = 0;

  const keyFor = (item) => {
    if (!item) return ++keySequence;
    if (item.queueId) return item.queueId;
    let key = keyMap.get(item);
    if (key === undefined) {
      key = ++keySequence;
      keyMap.set(item, key);
    }
    return key;
  };

  const onQueueLeave = (element) => {
    const { offsetTop, offsetLeft, offsetWidth } = element;
    element.style.top = `${offsetTop}px`;
    element.style.left = `${offsetLeft}px`;
    element.style.width = `${offsetWidth}px`;
  };

  watch(queueLength, (newLength, oldLength) => {
    if (oldLength !== undefined && Math.abs(newLength - oldLength) > 20) {
      disableQueueTransition.value = true;
      nextTick(() => {
        disableQueueTransition.value = false;
      });
    }
  });

  const rowIndexAt = (clientY) => {
    const rows = listContainer.value?.querySelectorAll('[data-queue-idx]') || [];
    for (const row of rows) {
      const rect = row.getBoundingClientRect();
      if (clientY >= rect.top && clientY <= rect.bottom) {
        return Number.parseInt(row.dataset.queueIdx, 10);
      }
    }
    if (rows.length > 0) {
      if (clientY < rows[0].getBoundingClientRect().top) return 0;
      if (clientY > rows[rows.length - 1].getBoundingClientRect().bottom) return rows.length - 1;
    }
    return -1;
  };

  const onMouseMove = (event) => {
    if (dragIndex.value === -1) return;
    event.preventDefault();
    const index = rowIndexAt(event.clientY);
    if (index !== -1) overIndex.value = index;
  };

  const finishDrag = (commit) => {
    if (
      commit &&
      dragIndex.value !== -1 &&
      overIndex.value !== -1 &&
      dragIndex.value !== overIndex.value
    ) {
      moveItem(dragIndex.value, overIndex.value);
    }
    dragIndex.value = -1;
    overIndex.value = -1;
    document.removeEventListener('mousemove', onMouseMove);
    document.removeEventListener('mouseup', onMouseUp);
    document.body.style.userSelect = '';
    document.body.style.cursor = '';
  };

  const onMouseUp = () => finishDrag(true);

  const onGripMouseDown = (index, event) => {
    event.preventDefault();
    event.stopPropagation();
    dragIndex.value = index;
    overIndex.value = index;
    document.body.style.userSelect = 'none';
    document.body.style.cursor = 'grabbing';
    document.addEventListener('mousemove', onMouseMove);
    document.addEventListener('mouseup', onMouseUp);
  };

  onUnmounted(() => finishDrag(false));

  return {
    dragIndex,
    overIndex,
    listContainer,
    disableQueueTransition,
    keyFor,
    onQueueLeave,
    onGripMouseDown,
  };
}
