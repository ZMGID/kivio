import { useCallback, useState } from 'react'

interface DrawerModel { providerId: string; model: string }

/** Provider selection and modal lifecycle, validated against the current provider set. */
export function useProviderModalController(providerIds: readonly string[]) {
  const [selected, setSelected] = useState('')
  const [picker, setPicker] = useState<string | null>(null)
  const [deleteRequest, setDeleteRequest] = useState<string | null>(null)
  const [drawer, setDrawer] = useState<DrawerModel | null>(null)
  const [test, setTest] = useState<string | null>(null)
  const exists = (id: string) => providerIds.includes(id)
  const selectedId = exists(selected) ? selected : (providerIds[0] ?? '')
  const pickerId = picker && exists(picker) ? picker : null
  const deleteId = deleteRequest && exists(deleteRequest) ? deleteRequest : null
  const drawerModel = drawer && exists(drawer.providerId) ? drawer : null
  const testId = test && exists(test) ? test : null

  // A newly added provider may be selected before its draft appears in this render.
  const select = useCallback((id: string) => setSelected(id), [])
  const openPicker = useCallback((id: string) => { if (providerIds.includes(id)) setPicker(id) }, [providerIds])
  const requestDelete = useCallback((id: string) => { if (providerIds.includes(id)) setDeleteRequest(id) }, [providerIds])
  const openDrawer = useCallback((value: DrawerModel) => { if (providerIds.includes(value.providerId)) setDrawer(value) }, [providerIds])
  const openTest = useCallback((id: string) => { if (providerIds.includes(id)) setTest(id) }, [providerIds])
  const closePicker = useCallback(() => setPicker(null), [])
  const cancelDelete = useCallback(() => setDeleteRequest(null), [])
  const closeDrawer = useCallback(() => setDrawer(null), [])
  const closeTest = useCallback(() => setTest(null), [])
  const providerDeleted = useCallback((id: string) => {
    if (picker === id) setPicker(null)
    if (deleteRequest === id) setDeleteRequest(null)
    if (drawer?.providerId === id) setDrawer(null)
    if (test === id) setTest(null)
    if (selected === id) setSelected('')
  }, [picker, deleteRequest, drawer, test, selected])
  const dismissTop = useCallback((): boolean => {
    if (pickerId) { setPicker(null); return true }
    if (deleteId) { setDeleteRequest(null); return true }
    return false
  }, [pickerId, deleteId])

  return {
    selectedId, pickerId, deleteId, drawerModel, testId,
    select, openPicker, requestDelete, openDrawer, openTest,
    closePicker, cancelDelete, closeDrawer, closeTest,
    providerDeleted, dismissTop,
  }
}
