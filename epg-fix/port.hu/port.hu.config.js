const axios = require('axios')
const dayjs = require('dayjs')

// Böngészőt utánzó fejlécek a 403-as letiltások megelőzésére
const headers = {
  'User-Agent':
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36',
  'Accept': 'application/json, text/plain, */*',
  'Accept-Language': 'hu-HU,hu;q=0.9,en-US;q=0.8,en;q=0.7',
  'Referer': 'https://port.hu/tv-musor',
  'Origin': 'https://port.hu'
}

module.exports = {
  site: 'port.hu',
  request: {
    headers
  },
  url({ channel, date }) {
    return `https://port.hu/tvapi?channel_id[]=tvchannel-${
      channel.site_id
    }&i_datetime_from=${date.format('YYYY-MM-DD')}&i_datetime_to=${date.format('YYYY-MM-DD')}`
  },
  parser({ content, channel }) {
    const items = parseItems(content, channel)

    let programs = []

    items.forEach(item => {
      programs.push({
        title: item.title,
        subtitle: item.episode_title,
        description: item.description || item.short_description,
        category: item.restriction?.category,
        start: dayjs.unix(item.start_ts),
        stop: dayjs(item.end_datetime)
      })
    })

    return programs
  },
  async channels() {
    const data = await axios
      .get('https://port.hu/tvapi/init-new', { headers })
      .then(r => r.data)
      .catch(console.error)

    if (!data || !data.channels) return []

    return data.channels.map(channel => {
      const [, site_id] = channel.id.split('-')

      return {
        site_id,
        name: channel.name,
        lang: 'hu'
      }
    })
  }
}

function parseItems(content, channel) {
  try {
    const data = JSON.parse(content)
    if (!data) return []

    const firstElement = Object.values(data)[0]
    if (!firstElement || !Array.isArray(firstElement.channels)) return []

    const channelData = firstElement.channels.find(
      _channel => _channel.id === `tvchannel-${channel.site_id}`
    )

    if (!channelData || !Array.isArray(channelData.programs)) return []

    return channelData.programs
  } catch {
    return []
  }
}