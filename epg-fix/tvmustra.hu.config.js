const cheerio = require('cheerio')
const axios = require('axios')
const dayjs = require('dayjs')
const utc = require('dayjs/plugin/utc')
const timezone = require('dayjs/plugin/timezone')
dayjs.extend(utc)
dayjs.extend(timezone)

module.exports = {
  site: 'tvmustra.hu',
  days: 2,
  request: {
    headers: {
      'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36',
      'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8',
      'Accept-Language': 'hu-HU,hu;q=0.9,en-US;q=0.8,en;q=0.7',
      'Referer': 'https://www.tvmustra.hu/',
      'Origin': 'https://www.tvmustra.hu'
    }
  },
  url({ channel, date }) {
    return `https://www.tvmustra.hu/tvmusor/${channel.site_id}/${date.format('YYYY-MM-DD')}`
  },
  parser({ content, date }) {
    const programs = []
    if (!content) return programs

    const items = parseItems(content)
    let curDate = date

    items.forEach(item => {
      const $item = cheerio.load(item)
      let start = parseStart($item, curDate)
      if (!start) return

      const prev = programs[programs.length - 1]
      if (prev) {
        if (start < prev.start) {
          start = start.add(1, 'day')
          curDate = curDate.add(1, 'day')
        }
        prev.stop = start
      }
      const stop = start.add(30, 'minute')
      const title = parseTitle($item)

      if (title) {
        programs.push({
          title,
          start,
          stop
        })
      }
    })

    return programs
  },
  async channels() {
    const html = await axios
      .get('https://www.tvmustra.hu/', {
        headers: {
          'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36',
          'Referer': 'https://www.tvmustra.hu/'
        }
      })
      .then(r => r.data)
      .catch(console.log)

    if (!html) return []

    const $ = cheerio.load(html)
    const items = $('select option').toArray()
    const channels = []

    items.forEach(item => {
      const name = $(item).text().trim()
      const site_id = $(item).attr('data-slug') || $(item).attr('value')

      if (!site_id || site_id === '#' || site_id === '') return

      const cleanSiteId = site_id.trim()
      if (!channels.find(c => c.site_id === cleanSiteId)) {
        channels.push({
          lang: 'hu',
          site_id: cleanSiteId,
          name
        })
      }
    })

    return channels
  }
}

function parseTitle($item) {
  let title = $item('.v-prog-title, div[class*="cim"], div[class*="title"], .v-prog-name').first().text().trim()

  if (!title) {
    title = $item.text().replace(/\b\d{2}:\d{2}\b/, '').trim()
  }

  return title
}

function parseStart($item, date) {
  let timeStr = $item('.v-prog-time, div[class*="idopont"], div[class*="time"]').first().text().trim()

  if (!timeStr) {
    const text = $item.text()
    const match = text.match(/\b(\d{2}:\d{2})\b/)
    if (match) {
      timeStr = match[1]
    }
  }

  if (!timeStr) {
    const onclick = $item.attr('onclick') || ''
    const match = onclick.match(/'(\d{2}:\d{2})'/)
    if (match) {
      timeStr = match[1]
    }
  }

  if (!timeStr) return null

  return dayjs.tz(`${date.format('YYYY-MM-DD')} ${timeStr}`, 'YYYY-MM-DD HH:mm', 'Europe/Budapest').utc()
}

function parseItems(content) {
  const $ = cheerio.load(content)

  let items = $('.v-prog-row').toArray()
  if (!items.length) {
    items = $('[onclick*="loadDetails"]').toArray()
  }
  if (!items.length) {
    items = $('div[data-page="channel"][data-show]').toArray()
  }

  return items
}